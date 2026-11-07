//! First-run setup: token, gating, single success under concurrency, CLI.

use axum::http::StatusCode;
use serde_json::json;

use super::support::{ADMIN_EMAIL, As, PASSWORD, TestApp};
use crate::accounts::setup::{self, SetupStatus};
use crate::config::Secret;

fn setup_body(token: &str, email: &str) -> serde_json::Value {
    json!({ "token": token, "email": email, "password": PASSWORD, "display_name": "Admin" })
}

#[tokio::test]
async fn setup_flow_creates_the_first_admin_once() {
    let app = TestApp::new().await;
    let resp = app.get("/api/v1/setup", &As::Anon).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body, json!({ "required": true }));

    let token = app.setup_token().await;
    // Wrong token.
    app.post(
        "/api/v1/setup",
        &As::Anon,
        setup_body("wrong-token", ADMIN_EMAIL),
    )
    .await
    .assert_error(StatusCode::FORBIDDEN, "setup_token_invalid");
    // Validation happens with the right token only, and doesn't consume it.
    app.post(
        "/api/v1/setup",
        &As::Anon,
        setup_body(&token, "not-an-email"),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let mut bad_url = setup_body(&token, ADMIN_EMAIL);
    bad_url["public_url"] = json!("https://sync.example.com/sub");
    let resp = app.post("/api/v1/setup", &As::Anon, bad_url).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "public_url");

    let resp = app
        .post(
            "/api/v1/setup",
            &As::Anon,
            json!({
                "token": token, "email": " Admin@Example.com ", "password": PASSWORD,
                "display_name": "Admin", "registration_mode": "open",
                "public_url": "https://Sync.Example.com/"
            }),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email"], ADMIN_EMAIL);
    assert_eq!(resp.body["is_admin"], true);
    assert_eq!(resp.body["email_verified"], true);
    let cookie = As::Cookie(resp.cookie().unwrap());
    app.get("/api/v1/me", &cookie).await.assert(StatusCode::OK);

    // Done: token cleared, no second setup, mode and public URL stored.
    assert_eq!(
        setup::status(&app.state.db).await.unwrap(),
        SetupStatus::Completed
    );
    assert_eq!(
        app.state.public_url().await.unwrap().as_deref(),
        Some("https://sync.example.com")
    );
    app.post(
        "/api/v1/setup",
        &As::Anon,
        setup_body(&token, "b@example.com"),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "setup_already_done");
    let resp = app.get("/api/v1/setup", &As::Anon).await;
    assert_eq!(resp.body, json!({ "required": false }));
    let resp = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(
        resp.body,
        json!({
            "setup_required": false, "registration_mode": "open",
            "password_requires_approval": true, "providers": []
        })
    );
}

#[tokio::test]
async fn api_is_gated_until_setup() {
    let app = TestApp::new().await;
    for (method, uri) in [
        ("GET", "/api/v1/me"),
        ("POST", "/api/v1/auth/login"),
        ("POST", "/api/v1/auth/register"),
        ("GET", "/api/v1/admin/users"),
        ("POST", "/api/v1/devices/token"),
    ] {
        app.call(method, uri, &As::Anon, Some(json!({})))
            .await
            .assert_error(StatusCode::CONFLICT, "setup_required");
    }
    app.get("/api/v1/health", &As::Anon)
        .await
        .assert(StatusCode::OK);
    let resp = app.get("/api/v1/auth/config", &As::Anon).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["setup_required"], true);
    assert_eq!(resp.body["registration_mode"], "invite");
    // Probes and unknown API paths are not gated.
    app.get("/health", &As::Anon).await.assert(StatusCode::OK);
    app.get("/api/v1/nope", &As::Anon)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    app.setup_admin().await;
    app.get("/api/v1/me", &As::Anon)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn concurrent_setup_has_exactly_one_winner() {
    let app = TestApp::new().await;
    let token = app.setup_token().await;
    let requests = (0..8).map(|i| {
        app.post(
            "/api/v1/setup",
            &As::Anon,
            setup_body(&token, &format!("admin{i}@example.com")),
        )
    });
    let responses = futures::future::join_all(requests).await;
    let winners = responses
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    assert_eq!(winners, 1, "{responses:?}");
    for resp in responses.iter().filter(|r| r.status != StatusCode::CREATED) {
        resp.assert_error(StatusCode::CONFLICT, "setup_already_done");
    }
    let (users, total) = crate::accounts::users::list(&app.state.db, None, 10, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert!(users[0].admin);
}

#[tokio::test]
async fn every_instance_sees_the_same_token_and_cli_prints_it() {
    let app = TestApp::new().await;
    let token = app.setup_token().await;
    // A second instance starting on the same database keeps the first token.
    let other = crate::db::connect(&app.cli_config()).await.unwrap();
    let status = setup::init(&other, None).await.unwrap();
    assert_eq!(status, SetupStatus::Pending(Secret::new(token.clone())));

    let config = app.cli_config();
    let printed = crate::cli::setup_token_text(&config).await.unwrap();
    assert_eq!(printed, format!("{token}\n"));

    app.setup_admin().await;
    let printed = crate::cli::setup_token_text(&config).await.unwrap();
    assert_eq!(printed, "setup is already completed\n");
    // After setup no instance creates a new token, even with SETUP_TOKEN set.
    let status = setup::init(&other, Some(&Secret::new("0123456789abcdef")))
        .await
        .unwrap();
    assert_eq!(status, SetupStatus::Completed);
}

#[tokio::test]
async fn setup_token_from_the_environment() {
    let app = TestApp::with(&[("SETUP_TOKEN", "my-fixed-setup-token")]).await;
    assert_eq!(app.setup_token().await, "my-fixed-setup-token");
    app.post(
        "/api/v1/setup",
        &As::Anon,
        setup_body("my-fixed-setup-token", ADMIN_EMAIL),
    )
    .await
    .assert(StatusCode::CREATED);
}

#[tokio::test]
async fn setup_is_rate_limited() {
    let app = TestApp::new().await;
    for _ in 0..10 {
        app.post("/api/v1/setup", &As::Anon, setup_body("wrong", ADMIN_EMAIL))
            .await
            .assert_error(StatusCode::FORBIDDEN, "setup_token_invalid");
    }
    let resp = app
        .post("/api/v1/setup", &As::Anon, setup_body("wrong", ADMIN_EMAIL))
        .await;
    resp.assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    assert!(resp.headers.contains_key("retry-after"));
}
