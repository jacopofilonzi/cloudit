use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::{AppState, router};
use crate::config::Config;

fn config_with_web_dir(web_dir: &Path) -> Config {
    let web_dir = web_dir.to_string_lossy().into_owned();
    Config::from_lookup(|key| (key == "WEB_DIR").then(|| web_dir.clone())).unwrap()
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
    let state = AppState {
        config: Arc::new(config_with_web_dir(web_dir)),
    };
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
    for uri in ["/health", "/ready", "/api/v1/health"] {
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
