//! Background jobs: admin API, immediate runs through the scheduler, lease exclusivity
//! between instances, interrupted runs, the cleanup job.

use std::time::Duration;

use axum::http::StatusCode;
use cloudit_core::protocol::{JobName, JobOutcome};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveValue::Set, ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::json;
use uuid::Uuid;

use super::support::{As, TestApp};
use crate::db::entities::{device_auth_codes, invites, job_runs, password_resets, sessions};
use crate::hooks::{self, Point};
use crate::jobs::scheduler::{self, Attempt};
use crate::jobs::store;
use crate::state::AppState;

const GRACE: Duration = Duration::from_secs(24 * 3600);

/// Runs of `job` recorded so far.
pub async fn runs(state: &AppState, job: JobName) -> Vec<job_runs::Model> {
    store::recent_runs(&state.db, job, 1000).await.unwrap()
}

/// Runs `job` now (lease-guarded, ignoring the schedule) and expects it to succeed.
pub async fn run_now(state: &AppState, job: JobName) -> crate::jobs::Counts {
    match scheduler::run_leased(state, job, true, Duration::ZERO)
        .await
        .unwrap()
    {
        Attempt::Ran {
            outcome: JobOutcome::Succeeded,
            counts,
            ..
        } => counts,
        other => panic!("{} did not succeed: {other:?}", job.as_str()),
    }
}

/// Waits until `job` has at least `n` finished runs.
async fn wait_runs(state: &AppState, job: JobName, n: usize) {
    for _ in 0..200 {
        let done = runs(state, job)
            .await
            .iter()
            .filter(|r| r.finished_at.is_some())
            .count();
        if done >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{} did not run {n} times", job.as_str());
}

#[tokio::test]
async fn admin_api_lists_jobs_and_requests_runs() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    app.get("/api/v1/admin/jobs", &user)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");

    let resp = app.get("/api/v1/admin/jobs", &admin).await;
    resp.assert(StatusCode::OK);
    let names: Vec<&str> = resp
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "retention",
            "gc",
            "orphan_sweep",
            "trash_purge",
            "cleanup",
            "update_check"
        ]
    );
    assert_eq!(resp.body[1]["interval_secs"], 3600);
    assert_eq!(resp.body[0]["requested_at"], json!(null));

    app.post("/api/v1/admin/jobs/nope/run", &admin, json!({}))
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    let resp = app
        .post("/api/v1/admin/jobs/cleanup/run", &admin, json!({}))
        .await;
    resp.assert(StatusCode::ACCEPTED);
    assert_eq!(resp.body["name"], "cleanup");
    assert!(resp.body["requested_at"].is_string());

    // A run consumes the request and is listed.
    run_now(&app.state, JobName::Cleanup).await;
    let resp = app.get("/api/v1/admin/jobs", &admin).await;
    let cleanup = &resp.body[4];
    assert_eq!(cleanup["requested_at"], json!(null));
    assert!(cleanup["last_started_at"].is_string());
    let run = &cleanup["recent_runs"][0];
    assert_eq!(run["outcome"], "succeeded");
    assert_eq!(run["instance_id"], app.state.leases.holder());
    assert!(run["finished_at"].is_string());
    assert_eq!(run["counts"]["sessions"], 0);
}

#[tokio::test]
async fn scheduler_runs_due_jobs_once_and_on_request() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let (second, _) = app.second_instance().await;
    // Two instances, each with its scheduler: every job (never run) is due once.
    scheduler::start_with(app.state.clone(), Duration::from_millis(20));
    scheduler::start_with(second.clone(), Duration::from_millis(20));
    for job in JobName::ALL {
        wait_runs(&app.state, job, 1).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    for job in JobName::ALL {
        assert_eq!(runs(&app.state, job).await.len(), 1, "{}", job.as_str());
    }

    // Run now: the request is noticed by a scheduler.
    app.post("/api/v1/admin/jobs/gc/run", &admin, json!({}))
        .await
        .assert(StatusCode::ACCEPTED);
    wait_runs(&app.state, JobName::Gc, 2).await;
    assert_eq!(runs(&app.state, JobName::Cleanup).await.len(), 1);
}

#[tokio::test]
async fn a_lease_lets_one_instance_run_a_job() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let (second, _) = app.second_instance().await;
    let pause = hooks::arm(
        Point::JobRunning,
        &format!("{}:cleanup", app.state.leases.holder()),
    );
    let first = {
        let state = app.state.clone();
        tokio::spawn(async move {
            scheduler::run_leased(&state, JobName::Cleanup, true, GRACE)
                .await
                .unwrap()
        })
    };
    pause.reached().await;
    // The first instance holds the lease: the second one skips, even when forced.
    assert_eq!(
        scheduler::run_leased(&second, JobName::Cleanup, true, GRACE)
            .await
            .unwrap(),
        Attempt::Skipped
    );
    pause.release();
    assert!(matches!(first.await.unwrap(), Attempt::Ran { .. }));
    // Released: the second instance may run it now (forced; it is not due).
    assert_eq!(
        scheduler::run_leased(&second, JobName::Cleanup, false, GRACE)
            .await
            .unwrap(),
        Attempt::Skipped
    );
    assert!(matches!(
        scheduler::run_leased(&second, JobName::Cleanup, true, GRACE)
            .await
            .unwrap(),
        Attempt::Ran { .. }
    ));
    let all = runs(&app.state, JobName::Cleanup).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].instance_id, second.leases.holder());
    assert_eq!(all[1].instance_id, app.state.leases.holder());
}

#[tokio::test]
async fn runs_left_running_are_marked_interrupted() {
    let app = TestApp::new().await;
    store::ensure_rows(&app.state.db).await.unwrap();
    let dead = Uuid::now_v7();
    job_runs::Entity::insert(job_runs::ActiveModel {
        id: Set(dead),
        job: Set("cleanup".to_owned()),
        instance_id: Set("gone".to_owned()),
        started_at: Set(crate::time::now()),
        finished_at: Set(None),
        outcome: Set("running".to_owned()),
        counts: Set("{}".to_owned()),
        error: Set(None),
    })
    .exec_without_returning(&app.state.db)
    .await
    .unwrap();
    run_now(&app.state, JobName::Cleanup).await;
    let old = job_runs::Entity::find_by_id(dead)
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.outcome, "interrupted");
    assert!(old.finished_at.is_some());
}

#[tokio::test]
async fn cleanup_deletes_what_can_never_be_used() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let user_id: Uuid = app.user_id(&user).await.parse().unwrap();
    let admin_id: Uuid = app.user_id(&admin).await.parse().unwrap();
    let db = &app.state.db;
    let now = crate::time::now();
    let past = now - chrono::TimeDelta::days(31);
    let token = || Uuid::now_v7().simple().to_string();

    // An expired session next to the user's live one.
    sessions::Entity::insert(sessions::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(token()),
        user_id: Set(user_id),
        created_at: Set(past),
        last_seen_at: Set(past),
        expires_at: Set(now - chrono::TimeDelta::seconds(1)),
        user_agent: Set(None),
    })
    .exec_without_returning(db)
    .await
    .unwrap();
    device_auth_codes::Entity::insert(device_auth_codes::ActiveModel {
        id: Set(Uuid::now_v7()),
        code_hash: Set(token()),
        user_id: Set(user_id),
        code_challenge: Set("x".repeat(43)),
        redirect_uri: Set("http://127.0.0.1:1/callback".to_owned()),
        device_name: Set("pc".to_owned()),
        os: Set("linux".to_owned()),
        token_ttl_secs: Set(Some(3600)),
        created_at: Set(past),
        expires_at: Set(past),
        used_at: Set(None),
    })
    .exec_without_returning(db)
    .await
    .unwrap();
    // A used reset and a live one.
    for used in [Some(now), None] {
        password_resets::Entity::insert(password_resets::ActiveModel {
            id: Set(Uuid::now_v7()),
            token_hash: Set(token()),
            user_id: Set(user_id),
            created_by: Set(None),
            created_at: Set(now),
            expires_at: Set(now + chrono::TimeDelta::hours(24)),
            used_at: Set(used),
        })
        .exec_without_returning(db)
        .await
        .unwrap();
    }
    // Invites: unused and long expired (deleted), used long ago (kept), unused and
    // recently expired (kept), live (kept).
    let invite = |expires_at, used_by: Option<Uuid>| invites::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(token()),
        created_by: Set(admin_id),
        email: Set(None),
        created_at: Set(past),
        expires_at: Set(expires_at),
        used_at: Set(used_by.map(|_| past)),
        used_by: Set(used_by),
        revoked_at: Set(None),
    };
    for model in [
        invite(past, None),
        invite(past, Some(user_id)),
        invite(now - chrono::TimeDelta::days(1), None),
        invite(now + chrono::TimeDelta::days(1), None),
    ] {
        invites::Entity::insert(model)
            .exec_without_returning(db)
            .await
            .unwrap();
    }
    // An old finished run (deleted).
    store::ensure_rows(db).await.unwrap();
    let old_run = store::start(db, JobName::Gc, "x", past).await.unwrap();
    store::finish(
        db,
        old_run,
        JobOutcome::Succeeded,
        &std::collections::BTreeMap::new(),
        None,
    )
    .await
    .unwrap();
    job_runs::Entity::update_many()
        .col_expr(job_runs::Column::StartedAt, Expr::value(past))
        .filter(job_runs::Column::Id.eq(old_run))
        .exec(db)
        .await
        .unwrap();

    let counts = run_now(&app.state, JobName::Cleanup).await;
    assert_eq!(counts.get("sessions"), 1);
    assert_eq!(counts.get("device_auth_codes"), 1);
    assert_eq!(counts.get("password_resets"), 1);
    assert_eq!(counts.get("invites"), 1);
    assert_eq!(counts.get("job_runs"), 1);
    assert_eq!(invites::Entity::find().all(db).await.unwrap().len(), 3);
    assert!(
        invites::Entity::find()
            .filter(invites::Column::UsedBy.eq(user_id))
            .one(db)
            .await
            .unwrap()
            .is_some(),
        "used invites are kept"
    );
    // The live session still works.
    app.get("/api/v1/me", &user).await.assert(StatusCode::OK);
    let _ = As::Anon;
}
