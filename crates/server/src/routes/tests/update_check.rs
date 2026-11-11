//! The `update_check` job against a fake tag endpoint on 127.0.0.1 (never GitHub),
//! and what `GET /admin/summary` then tells admins.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use cloudit_core::protocol::{JobName, JobOutcome};
use serde_json::{Value, json};

use super::jobs::run_now;
use super::support::{As, TestApp};
use crate::accounts::settings;
use crate::jobs::scheduler::{self, Attempt};
use crate::jobs::update_check::SERVER_CHANGELOG_URL;

/// What the fake answers, and the `User-Agent` it last saw.
#[derive(Default)]
struct Fake {
    status: u16,
    body: String,
    user_agent: Option<String>,
    hits: usize,
}

type Shared = Arc<Mutex<Fake>>;

async fn tags(State(fake): State<Shared>, headers: HeaderMap) -> impl IntoResponse {
    let mut fake = fake.lock().unwrap();
    fake.hits += 1;
    fake.user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    (
        StatusCode::from_u16(fake.status).unwrap(),
        [("content-type", "application/json")],
        fake.body.clone(),
    )
}

/// Starts the fake; returns its URL.
async fn start(fake: &Shared) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/git/matching-refs/tags/server-v",
        listener.local_addr().unwrap()
    );
    let app = Router::new()
        .route("/git/matching-refs/tags/server-v", get(tags))
        .with_state(Arc::clone(fake));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    url
}

fn answer(fake: &Shared, status: u16, body: Value) {
    let mut fake = fake.lock().unwrap();
    fake.status = status;
    fake.body = body.to_string();
}

/// A ref object as GitHub's "list matching references" answers it.
fn git_ref(name: &str) -> Value {
    json!({
        "ref": name,
        "node_id": "REF_kwDOtest",
        "url": format!("https://api.github.com/repos/o/r/git/{name}"),
        "object": {"sha": "0123456789abcdef0123456789abcdef01234567", "type": "commit",
                   "url": "https://api.github.com/repos/o/r/git/commits/0123"}
    })
}

fn refs(names: &[&str]) -> Value {
    Value::Array(names.iter().map(|n| git_ref(n)).collect())
}

async fn run(app: &TestApp) -> (JobOutcome, crate::jobs::Counts) {
    match scheduler::run_leased(&app.state, JobName::UpdateCheck, true, Duration::ZERO)
        .await
        .unwrap()
    {
        Attempt::Ran {
            outcome, counts, ..
        } => (outcome, counts),
        Attempt::Skipped => panic!("skipped"),
    }
}

#[tokio::test]
async fn stores_the_latest_version_and_tells_admins() {
    let fake = Shared::default();
    let url = start(&fake).await;
    let app = TestApp::with(&[("UPDATE_CHECK", "true"), ("UPDATE_CHECK_URL", &url)]).await;
    let admin = app.setup_admin().await;

    // Never checked: no release, nothing to update.
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["server_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(resp.body["latest_release"], json!(null));
    assert_eq!(resp.body["update_available"], false);

    // The highest plain server tag wins; app tags, old `v` tags, pre-release suffixes
    // and other names are ignored.
    answer(
        &fake,
        200,
        refs(&[
            "refs/tags/server-v1000.0.0-rc.1",
            "refs/tags/server-v998.0.0",
            "refs/tags/server-v999.0.0",
            "refs/tags/server-v99.0.0",
            "refs/tags/server-vnext",
        ]),
    );
    let counts = run_now(&app.state, JobName::UpdateCheck).await;
    assert_eq!(counts.get("checked"), 1);
    assert_eq!(counts.get("newer"), 1);
    let agent = fake.lock().unwrap().user_agent.clone().unwrap();
    assert!(agent.starts_with("cloudit-server/"), "{agent}");

    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(resp.body["latest_release"]["version"], "999.0.0");
    assert_eq!(resp.body["latest_release"]["url"], SERVER_CHANGELOG_URL);
    assert!(resp.body["latest_release"]["checked_at"].is_string());
    assert_eq!(resp.body["update_available"], true);

    // The same version as the server: stored, but nothing to update.
    let current = format!("refs/tags/server-v{}", env!("CARGO_PKG_VERSION"));
    answer(&fake, 200, refs(&[&current]));
    let counts = run_now(&app.state, JobName::UpdateCheck).await;
    assert_eq!(counts.get("newer"), 0);
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(
        resp.body["latest_release"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(resp.body["update_available"], false);
}

#[tokio::test]
async fn bad_answers_fail_the_run_and_keep_the_previous_value() {
    let fake = Shared::default();
    let url = start(&fake).await;
    let app = TestApp::with(&[("UPDATE_CHECK", "true"), ("UPDATE_CHECK_URL", &url)]).await;
    app.setup_admin().await;
    answer(&fake, 200, refs(&["refs/tags/server-v1.2.3"]));
    run_now(&app.state, JobName::UpdateCheck).await;

    for (status, body) in [
        (500, json!({"message": "boom"})),
        (200, json!({"unexpected": true})),
        (200, json!([{"unexpected": true}])),
        (200, json!([{"ref": 5}])),
    ] {
        answer(&fake, status, body.clone());
        let (outcome, _) = run(&app).await;
        assert_eq!(outcome, JobOutcome::Failed, "{status} {body}");
    }
    let kept = settings::latest_release(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.version, "1.2.3");

    // No server version published yet (no matching tag, a 404, only other tags): a
    // success that stores nothing new.
    for (status, body) in [
        (404, json!({"message": "Not Found"})),
        (200, json!([])),
        (200, refs(&["refs/tags/server-v3.0.0-beta"])),
        (200, refs(&["refs/tags/server-vnightly"])),
    ] {
        answer(&fake, status, body.clone());
        let (outcome, counts) = run(&app).await;
        assert_eq!(outcome, JobOutcome::Succeeded, "{status} {body}");
        assert_eq!(counts.get("no_release"), 1, "{status} {body}");
    }
    let kept = settings::latest_release(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.version, "1.2.3");
}

#[tokio::test]
async fn disabled_checks_contact_nothing() {
    let fake = Shared::default();
    let url = start(&fake).await;
    let app = TestApp::with(&[("UPDATE_CHECK", "false"), ("UPDATE_CHECK_URL", &url)]).await;
    app.setup_admin().await;
    let counts = run_now(&app.state, JobName::UpdateCheck).await;
    assert_eq!(counts.get("disabled"), 1);
    assert_eq!(fake.lock().unwrap().hits, 0);
    assert!(
        settings::latest_release(&app.state.db)
            .await
            .unwrap()
            .is_none()
    );
}

/// Moves the job's last start `secs` seconds into the past.
async fn started_ago(app: &TestApp, secs: i64) {
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
    let at = crate::time::now() - chrono::Duration::seconds(secs);
    crate::db::entities::jobs::Entity::update_many()
        .col_expr(
            crate::db::entities::jobs::Column::LastStartedAt,
            Expr::value(at),
        )
        .filter(crate::db::entities::jobs::Column::Name.eq("update_check"))
        .exec(&app.state.db)
        .await
        .unwrap();
}

async fn requested(app: &TestApp) -> bool {
    crate::jobs::store::get(&app.state.db, JobName::UpdateCheck)
        .await
        .unwrap()
        .is_some_and(|row| row.requested_at.is_some())
}

#[tokio::test]
async fn the_summary_requests_a_check_at_most_every_ten_minutes() {
    let fake = Shared::default();
    let url = start(&fake).await;
    let app = TestApp::with(&[("UPDATE_CHECK", "true"), ("UPDATE_CHECK_URL", &url)]).await;
    let admin = app.setup_admin().await;
    answer(&fake, 200, refs(&["refs/tags/server-v999.0.0"]));

    // Never checked: reading the summary requests a check (and doesn't wait for it).
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["update_check"]["enabled"], true);
    assert_eq!(resp.body["update_check"]["pending"], true);
    assert_eq!(resp.body["update_check"]["last_outcome"], json!(null));
    assert!(requested(&app).await);
    assert_eq!(fake.lock().unwrap().hits, 0);

    // A scheduler runs it; then the summary shows the result and requests nothing.
    run_now(&app.state, JobName::UpdateCheck).await;
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(resp.body["update_check"]["pending"], false);
    assert_eq!(resp.body["update_check"]["last_outcome"], "succeeded");
    assert!(resp.body["update_check"]["last_finished_at"].is_string());
    assert_eq!(resp.body["update_available"], true);
    assert!(!requested(&app).await);

    // Within ten minutes of the last start: nothing; after: a new request.
    started_ago(&app, 9 * 60).await;
    app.get("/api/v1/admin/summary", &admin).await;
    assert!(!requested(&app).await);
    started_ago(&app, 10 * 60 + 1).await;
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(resp.body["update_check"]["pending"], true);
    assert!(requested(&app).await);
}

#[tokio::test]
async fn check_now_is_limited_to_once_a_minute() {
    let fake = Shared::default();
    let url = start(&fake).await;
    let app = TestApp::with(&[("UPDATE_CHECK", "true"), ("UPDATE_CHECK_URL", &url)]).await;
    let admin = app.setup_admin().await;
    answer(&fake, 200, json!([]));

    let resp = app
        .post("/api/v1/admin/update-check", &admin, json!({}))
        .await;
    resp.assert(StatusCode::ACCEPTED);
    assert_eq!(resp.body["update_check"]["pending"], true);
    // Already requested: accepted again (the same check is coming).
    app.post("/api/v1/admin/update-check", &admin, json!({}))
        .await
        .assert(StatusCode::ACCEPTED);

    // Just ran: refused for the rest of the minute, here and through the jobs route.
    run_now(&app.state, JobName::UpdateCheck).await;
    started_ago(&app, 20).await;
    let resp = app
        .post("/api/v1/admin/update-check", &admin, json!({}))
        .await;
    resp.assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    let retry: u64 = resp.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((39..=41).contains(&retry), "{retry}");
    app.post("/api/v1/admin/jobs/update_check/run", &admin, json!({}))
        .await
        .assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    assert!(!requested(&app).await);
    // Other jobs are not limited.
    app.post("/api/v1/admin/jobs/cleanup/run", &admin, json!({}))
        .await
        .assert(StatusCode::ACCEPTED);

    started_ago(&app, 61).await;
    app.post("/api/v1/admin/update-check", &admin, json!({}))
        .await
        .assert(StatusCode::ACCEPTED);
    assert!(requested(&app).await);
}

#[tokio::test]
async fn check_now_is_for_admins_only() {
    let app = TestApp::with(&[
        ("UPDATE_CHECK", "true"),
        ("UPDATE_CHECK_URL", "http://127.0.0.1:9/x"),
    ])
    .await;
    app.setup_admin().await;
    let user = app.user("someone@example.test").await;
    app.post("/api/v1/admin/update-check", &user, json!({}))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    app.post("/api/v1/admin/update-check", &As::Anon, json!({}))
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    app.get("/api/v1/admin/summary", &user)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    assert!(!requested(&app).await);
}

#[tokio::test]
async fn disabled_checks_are_never_requested() {
    let app = TestApp::with(&[("UPDATE_CHECK", "false")]).await;
    let admin = app.setup_admin().await;
    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(resp.body["update_check"]["enabled"], false);
    assert_eq!(resp.body["update_check"]["pending"], false);
    let resp = app
        .post("/api/v1/admin/update-check", &admin, json!({}))
        .await;
    resp.assert(StatusCode::ACCEPTED);
    assert_eq!(resp.body["update_check"]["enabled"], false);
    assert_eq!(resp.body["update_check"]["pending"], false);
    assert!(!requested(&app).await);
}
