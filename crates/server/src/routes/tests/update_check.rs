//! The `update_check` job against a fake release endpoint on 127.0.0.1 (never GitHub),
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
use super::support::TestApp;
use crate::accounts::settings;
use crate::jobs::scheduler::{self, Attempt};

/// What the fake answers, and the `User-Agent` it last saw.
#[derive(Default)]
struct Fake {
    status: u16,
    body: String,
    user_agent: Option<String>,
    hits: usize,
}

type Shared = Arc<Mutex<Fake>>;

async fn releases(State(fake): State<Shared>, headers: HeaderMap) -> impl IntoResponse {
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
    let url = format!("http://{}/releases", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/releases", get(releases))
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
async fn stores_the_latest_release_and_tells_admins() {
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

    // The newest published server release wins; app releases (GitHub's "latest"),
    // drafts, pre-releases and old `v` tags are ignored.
    let page = "https://github.com/jacopofilonzi/cloudit/releases/tag/server-v999.0.0";
    answer(
        &fake,
        200,
        json!([
            {"tag_name": "app-v1000.0.0", "html_url": "https://example.test/app", "draft": false, "prerelease": false},
            {"tag_name": "server-v1001.0.0", "html_url": "https://example.test/draft", "draft": true, "prerelease": false},
            {"tag_name": "server-v1002.0.0", "html_url": "https://example.test/rc", "draft": false, "prerelease": true},
            {"tag_name": "v1003.0.0", "html_url": "https://example.test/old", "draft": false, "prerelease": false},
            {"tag_name": "server-v998.0.0", "html_url": "https://example.test/998", "draft": false, "prerelease": false},
            {"tag_name": "server-v999.0.0", "html_url": page, "name": "cloudit server 999", "draft": false, "prerelease": false}
        ]),
    );
    let counts = run_now(&app.state, JobName::UpdateCheck).await;
    assert_eq!(counts.get("checked"), 1);
    assert_eq!(counts.get("newer"), 1);
    let agent = fake.lock().unwrap().user_agent.clone().unwrap();
    assert!(agent.starts_with("cloudit-server/"), "{agent}");

    let resp = app.get("/api/v1/admin/summary", &admin).await;
    assert_eq!(resp.body["latest_release"]["version"], "999.0.0");
    assert_eq!(resp.body["latest_release"]["url"], page);
    assert!(resp.body["latest_release"]["checked_at"].is_string());
    assert_eq!(resp.body["update_available"], true);

    // The same version as the server: stored, but nothing to update.
    let current = format!("server-v{}", env!("CARGO_PKG_VERSION"));
    answer(&fake, 200, json!([{"tag_name": current, "html_url": page}]));
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
    let page = "https://example.test/releases/v1.2.3";
    answer(
        &fake,
        200,
        json!([{"tag_name": "server-v1.2.3", "html_url": page}]),
    );
    run_now(&app.state, JobName::UpdateCheck).await;

    for (status, body) in [
        (500, json!({"message": "boom"})),
        (200, json!({"unexpected": true})),
        (200, json!([{"unexpected": true}])),
        (
            200,
            json!([{"tag_name": "server-v2.0.0", "html_url": "javascript:alert(1)"}]),
        ),
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

    // No server release published yet (none at all, a 404, only other tags): a
    // success that stores nothing new.
    for (status, body) in [
        (404, json!({"message": "Not Found"})),
        (200, json!([])),
        (200, json!([{"tag_name": "app-v3.0.0", "html_url": page}])),
        (200, json!([{"tag_name": "nightly", "html_url": page}])),
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
