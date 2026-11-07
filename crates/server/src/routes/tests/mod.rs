mod admin;
mod approval;
mod archive;
mod auth;
mod chunks;
mod devices;
mod fake_idp;
mod folders;
mod history;
mod me;
mod oauth;
mod onboarding;
mod presign;
mod providers;
mod quota;
mod scopes;
mod secret_key;
mod setup;
mod snapshots;
mod support;
mod sync_client;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::{AppState, router};
use crate::config::Config;

/// A full state (SQLite + fs storage in `data_dir`) serving the web UI from `web_dir`.
async fn state_with(web_dir: &Path, data_dir: &Path, extra: &[(&str, &str)]) -> AppState {
    let mut vars: Vec<(String, String)> = vec![
        ("WEB_DIR".to_owned(), web_dir.to_string_lossy().into_owned()),
        (
            "DATA_DIR".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        ),
    ];
    vars.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    let config =
        Config::from_lookup(|key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
            .unwrap();
    AppState::init(config).await.unwrap()
}

/// A scratch directory unique to one test, removed on drop.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cloudit-server-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn get(web_dir: &Path, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let data = tempfile::tempdir().unwrap();
    let state = state_with(web_dir, data.path(), &[]).await;
    get_with(state, uri).await
}

async fn get_with(state: AppState, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let response = router(state)
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, content_type, body.to_vec())
}

fn missing_dir() -> PathBuf {
    std::env::temp_dir().join("cloudit-server-test-does-not-exist")
}

#[tokio::test]
async fn health_endpoints_report_ok_and_version() {
    for uri in ["/health", "/api/v1/health"] {
        let (status, content_type, body) = get(&missing_dir(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(content_type.as_deref(), Some("application/json"), "{uri}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok", "{uri}");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"), "{uri}");
    }
}

#[tokio::test]
async fn unknown_api_route_is_json_not_found() {
    let web = ScratchDir::new("api-404");
    std::fs::write(web.0.join("index.html"), "<html>spa</html>").unwrap();
    for uri in ["/api/v1/nope", "/api/nope", "/api"] {
        let (status, _, body) = get(&web.0, uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "not_found", "{uri}");
    }
}

#[tokio::test]
async fn spa_serves_files_and_falls_back_to_index() {
    let web = ScratchDir::new("spa");
    std::fs::write(web.0.join("index.html"), "<html>spa</html>").unwrap();
    std::fs::write(web.0.join("app.js"), "console.info(1)").unwrap();

    let (status, _, body) = get(&web.0, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"<html>spa</html>");

    let (status, _, body) = get(&web.0, "/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"console.info(1)");

    let (status, _, body) = get(&web.0, "/settings/devices").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"<html>spa</html>");
}

#[tokio::test]
async fn missing_web_dir_returns_json_not_found() {
    let (status, _, body) = get(&missing_dir(), "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "not_found");
}

#[tokio::test]
async fn ready_reports_components() {
    let data = tempfile::tempdir().unwrap();
    let state = state_with(&missing_dir(), data.path(), &[]).await;
    let (status, _, body) = get_with(state, "/ready").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "components": { "database": "ok", "storage": "ok", "redis": "disabled" }
        })
    );
    // The default SQLite database and blob directory live under DATA_DIR.
    assert!(data.path().join("cloudit.db").is_file());
    assert!(data.path().join("blobs").is_dir());
}

#[tokio::test]
async fn ready_fails_without_storage_but_not_without_redis() {
    let data = tempfile::tempdir().unwrap();
    let state = state_with(
        &missing_dir(),
        data.path(),
        &[("REDIS_URL", "redis://127.0.0.1:1")],
    )
    .await;
    // Redis down alone: still ready.
    let (status, _, body) = get_with(state.clone(), "/ready").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["components"]["redis"], "unavailable");

    // Storage gone: not ready.
    std::fs::remove_dir_all(data.path().join("blobs")).unwrap();
    let (status, _, body) = get_with(state, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "unavailable");
    assert_eq!(json["components"]["storage"], "unavailable");
    assert_eq!(json["components"]["database"], "ok");
}
