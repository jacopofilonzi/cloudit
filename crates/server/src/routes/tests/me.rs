//! `/me`: profile updates, password change, sessions.

use axum::http::StatusCode;
use serde_json::json;

use super::support::{ADMIN_EMAIL, As, PASSWORD, TestApp};

#[tokio::test]
async fn profile_and_display_name() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let resp = app.get("/api/v1/me", &admin).await;
    resp.assert(StatusCode::OK);
    let keys: Vec<&str> = resp
        .body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "created_at",
            "display_name",
            "email",
            "email_verified",
            "has_password",
            "id",
            "is_admin"
        ]
    );
    assert!(
        !resp.body.to_string().contains("argon2"),
        "no hash in responses"
    );

    let resp = app
        .call(
            "PATCH",
            "/api/v1/me",
            &admin,
            Some(json!({ "display_name": "  Ada " })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["display_name"], "Ada");
    app.call(
        "PATCH",
        "/api/v1/me",
        &admin,
        Some(json!({ "display_name": "" })),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
}

#[tokio::test]
async fn email_change_rules() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    app.user("taken@example.com").await;
    let patch = |body: serde_json::Value| app.call("PATCH", "/api/v1/me", &admin, Some(body));

    let resp = patch(json!({ "email": "new@example.com" })).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "current_password");
    patch(json!({ "email": "new@example.com", "current_password": "wrong password" }))
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    patch(json!({ "email": "taken@example.com", "current_password": PASSWORD }))
        .await
        .assert_error(StatusCode::CONFLICT, "email_taken");
    // Same email (case-insensitively): nothing to change, no password needed.
    let resp = patch(json!({ "email": "ADMIN@example.com" })).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["email_verified"], true);

    let resp = patch(json!({ "email": "New@Example.com", "current_password": PASSWORD })).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["email"], "new@example.com");
    assert_eq!(resp.body["email_verified"], false);
    app.login("new@example.com", PASSWORD)
        .await
        .assert(StatusCode::OK);
    app.login(ADMIN_EMAIL, PASSWORD)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
}

#[tokio::test]
async fn password_change_keeps_only_the_current_session() {
    let app = TestApp::new().await;
    let current = app.setup_admin().await;
    let other = As::Cookie(app.login(ADMIN_EMAIL, PASSWORD).await.cookie().unwrap());
    let change = |body: serde_json::Value| app.post("/api/v1/me/password", &current, body);

    change(json!({ "new_password": "a brand new password" }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    change(json!({ "current_password": "wrong password", "new_password": "a brand new password" }))
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    change(json!({ "current_password": PASSWORD, "new_password": "short" }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    change(json!({ "current_password": PASSWORD, "new_password": "a brand new password" }))
        .await
        .assert(StatusCode::NO_CONTENT);

    app.get("/api/v1/me", &current).await.assert(StatusCode::OK);
    app.get("/api/v1/me", &other)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    app.login(ADMIN_EMAIL, "a brand new password")
        .await
        .assert(StatusCode::OK);
}

#[tokio::test]
async fn password_checks_are_rate_limited() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let wrong =
        json!({ "current_password": "wrong password", "new_password": "a brand new password" });
    for _ in 0..10 {
        app.post("/api/v1/me/password", &admin, wrong.clone())
            .await
            .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    }
    app.post("/api/v1/me/password", &admin, wrong)
        .await
        .assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
}

#[tokio::test]
async fn sessions_list_and_revoke() {
    let app = TestApp::new().await;
    let current = app.setup_admin().await;
    let other = As::Cookie(app.login(ADMIN_EMAIL, PASSWORD).await.cookie().unwrap());
    let someone_else = app.user("user@example.com").await;

    let resp = app.get("/api/v1/me/sessions", &current).await;
    resp.assert(StatusCode::OK);
    let list = resp.body.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list.iter().filter(|s| s["current"] == true).count(), 1);
    assert_eq!(list[0]["user_agent"], "cloudit-test");
    let other_id = list.iter().find(|s| s["current"] == false).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let current_id = list.iter().find(|s| s["current"] == true).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Someone else's session id: not found.
    app.call(
        "DELETE",
        &format!("/api/v1/me/sessions/{other_id}"),
        &someone_else,
        None,
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call("DELETE", "/api/v1/me/sessions/not-a-uuid", &current, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    app.call(
        "DELETE",
        &format!("/api/v1/me/sessions/{other_id}"),
        &current,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.get("/api/v1/me", &other)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");

    let resp = app
        .call(
            "DELETE",
            &format!("/api/v1/me/sessions/{current_id}"),
            &current,
            None,
        )
        .await;
    resp.assert(StatusCode::NO_CONTENT);
    assert_eq!(
        resp.cookie().as_deref(),
        Some(""),
        "current session: cookie cleared"
    );
    app.get("/api/v1/me", &current)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}
