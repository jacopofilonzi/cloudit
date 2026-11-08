//! Device login (loopback + PKCE), bearer tokens, device management.

use axum::http::StatusCode;
use sea_orm::EntityTrait as _;
use sea_orm::sea_query::Expr;
use serde_json::{Value, json};

use super::support::{As, TestApp};
use crate::auth::tokens::pkce_challenge;
use crate::db::entities::{device_auth_codes, devices};

const REDIRECT: &str = "http://127.0.0.1:53682/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn authorize_body(ttl: Option<&str>) -> Value {
    let mut body = json!({
        "code_challenge": pkce_challenge(VERIFIER),
        "code_challenge_method": "S256",
        "redirect_uri": REDIRECT,
        "state": "xyz",
        "device_name": "Gaming PC",
        "os": "windows",
    });
    if let Some(ttl) = ttl {
        body["token_ttl"] = json!(ttl);
    }
    body
}

/// Runs `authorize`; returns the code from the redirect URL.
async fn authorize(app: &TestApp, who: &As, body: Value) -> String {
    let resp = app.post("/api/v1/devices/authorize", who, body).await;
    resp.assert(StatusCode::OK);
    let url = url::Url::parse(resp.body["redirect_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.path(), "/callback");
    let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(query[1], ("state".to_owned(), "xyz".to_owned()));
    query[0].1.clone()
}

async fn exchange(
    app: &TestApp,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> super::support::Resp {
    // Native client: no cookie, no Origin.
    let request = axum::http::Request::post("/api/v1/devices/token")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "code": code, "code_verifier": verifier, "redirect_uri": redirect })
                .to_string(),
        ))
        .unwrap();
    app.send(request).await
}

/// A signed-in device: (user cookie, device token, device id).
pub(super) async fn device(app: &TestApp) -> (As, String, String) {
    let user = app.user("user@example.com").await;
    let code = authorize(app, &user, authorize_body(None)).await;
    let resp = exchange(app, &code, VERIFIER, REDIRECT).await;
    resp.assert(StatusCode::OK);
    (
        user,
        resp.body["token"].as_str().unwrap().to_owned(),
        resp.body["device_id"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn pkce_login_issues_a_device_token() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let code = authorize(&app, &user, authorize_body(Some("7d"))).await;

    let resp = exchange(&app, &code, VERIFIER, REDIRECT).await;
    resp.assert(StatusCode::OK);
    let token = resp.body["token"].as_str().unwrap();
    assert!(token.starts_with("cld_"));
    let expires =
        chrono::DateTime::parse_from_rfc3339(resp.body["expires_at"].as_str().unwrap()).unwrap();
    let days = (expires.with_timezone(&chrono::Utc) - crate::time::now()).num_hours();
    assert!((7 * 24 - 1..=7 * 24).contains(&days), "{days}");

    // Only the hash is stored.
    let stored = devices::Entity::find()
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stored.token_hash, token);
    assert_eq!(stored.name, "Gaming PC");
    assert_eq!(stored.os, "windows");

    // The code is single use.
    exchange(&app, &code, VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");

    // The bearer token works on GET /me.
    let bearer = As::Bearer(token.to_owned());
    let resp = app.get("/api/v1/me", &bearer).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["email"], "user@example.com");
}

#[tokio::test]
async fn wrong_verifier_or_redirect_burns_the_code() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;

    let code = authorize(&app, &user, authorize_body(None)).await;
    let wrong = "x".repeat(43);
    exchange(&app, &code, &wrong, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
    exchange(&app, &code, VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");

    let code = authorize(&app, &user, authorize_body(None)).await;
    exchange(&app, &code, VERIFIER, "http://127.0.0.1:53683/callback")
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
    exchange(&app, &code, VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");

    exchange(&app, "made-up", VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
}

#[tokio::test]
async fn expired_code_is_refused() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let code = authorize(&app, &user, authorize_body(None)).await;
    device_auth_codes::Entity::update_many()
        .col_expr(
            device_auth_codes::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&app.state.db)
        .await
        .unwrap();
    exchange(&app, &code, VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
}

#[tokio::test]
async fn authorize_validates_its_input() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let post = |body: Value| app.post("/api/v1/devices/authorize", &user, body);

    for redirect in [
        "http://localhost:53682/cb",
        "https://127.0.0.1:53682/cb",
        "http://127.0.0.1/cb",
        "http://example.com:53682/cb",
        "http://127.0.0.1:53682/cb?x=1",
    ] {
        let mut body = authorize_body(None);
        body["redirect_uri"] = json!(redirect);
        let resp = post(body).await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(resp.body["field"], "redirect_uri", "{redirect}");
    }
    let mut body = authorize_body(None);
    body["redirect_uri"] = json!("http://[::1]:53682/cb");
    post(body).await.assert(StatusCode::OK);

    for ttl in ["1h", "1d", "7d", "30d", "1y"] {
        post(authorize_body(Some(ttl))).await.assert(StatusCode::OK);
    }
    post(authorize_body(Some("2d")))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");

    let mut body = authorize_body(None);
    body["code_challenge_method"] = json!("plain");
    post(body)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    let mut body = authorize_body(None);
    body["code_challenge"] = json!("too-short");
    post(body)
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let mut body = authorize_body(None);
    body["state"] = json!("");
    post(body)
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");

    // Needs a web session.
    app.post("/api/v1/devices/authorize", &As::Anon, authorize_body(None))
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn bearer_tokens_are_accepted_only_where_allowed() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let (_, token, id) = device(&app).await;
    let bearer = As::Bearer(token);

    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);
    for (method, uri) in [
        ("PATCH", "/api/v1/me".to_owned()),
        ("POST", "/api/v1/me/password".to_owned()),
        ("GET", "/api/v1/me/sessions".to_owned()),
        ("GET", "/api/v1/devices".to_owned()),
        ("POST", "/api/v1/devices/authorize".to_owned()),
        ("DELETE", format!("/api/v1/devices/{id}")),
        ("GET", "/api/v1/admin/users".to_owned()),
    ] {
        app.call(method, &uri, &bearer, Some(json!({})))
            .await
            .assert_error(StatusCode::FORBIDDEN, "forbidden");
    }
    // A bearer request skips CSRF but is not a session: logout has nothing to delete.
    app.call("POST", "/api/v1/auth/logout", &bearer, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);

    // Unknown or malformed tokens.
    for token in ["cld_nope", "not-a-device-token"] {
        app.get("/api/v1/me", &As::Bearer(token.to_owned()))
            .await
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
}

#[tokio::test]
async fn devices_list_rename_revoke_and_expiry() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let (user, token, id) = device(&app).await;
    let bearer = As::Bearer(token);

    // Using the token records last_seen_at (at most every few minutes).
    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);
    let resp = app.get("/api/v1/devices", &user).await;
    resp.assert(StatusCode::OK);
    let list = resp.body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], id.as_str());
    assert!(list[0]["last_seen_at"].is_string());
    assert!(list[0]["revoked_at"].is_null());

    let resp = app
        .call(
            "PATCH",
            &format!("/api/v1/devices/{id}"),
            &user,
            Some(json!({ "name": "Laptop" })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Laptop");

    // Another user can't see or touch it.
    let other = app.user("other@example.com").await;
    let resp = app.get("/api/v1/devices", &other).await;
    assert_eq!(resp.body, json!([]));
    app.call("DELETE", &format!("/api/v1/devices/{id}"), &other, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    app.call("DELETE", &format!("/api/v1/devices/{id}"), &user, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    app.get("/api/v1/me", &bearer)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    let resp = app.get("/api/v1/devices", &user).await;
    assert!(resp.body[0]["revoked_at"].is_string(), "revocation is soft");

    // Expiry.
    let code = authorize(&app, &user, authorize_body(Some("1h"))).await;
    let resp = exchange(&app, &code, VERIFIER, REDIRECT).await;
    let bearer = As::Bearer(resp.body["token"].as_str().unwrap().to_owned());
    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);
    devices::Entity::update_many()
        .col_expr(
            devices::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&app.state.db)
        .await
        .unwrap();
    app.get("/api/v1/me", &bearer)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn disabled_user_devices_stop_working() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let (user, token, _) = device(&app).await;
    let id = app.user_id(&user).await;
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{id}"),
        &admin,
        Some(json!({ "disabled": true })),
    )
    .await
    .assert(StatusCode::OK);
    app.get("/api/v1/me", &As::Bearer(token))
        .await
        .assert_error(StatusCode::FORBIDDEN, "account_disabled");
}

#[tokio::test]
async fn token_endpoint_is_rate_limited() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    for _ in 0..30 {
        exchange(&app, "made-up", VERIFIER, REDIRECT)
            .await
            .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
    }
    exchange(&app, "made-up", VERIFIER, REDIRECT)
        .await
        .assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
}

#[tokio::test]
async fn forever_tokens_never_expire_until_revoked_or_disabled() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let code = authorize(&app, &user, authorize_body(Some("forever"))).await;
    let resp = exchange(&app, &code, VERIFIER, REDIRECT).await;
    resp.assert(StatusCode::OK);
    assert!(resp.body["expires_at"].is_null());
    let id = resp.body["device_id"].as_str().unwrap().to_owned();
    let bearer = As::Bearer(resp.body["token"].as_str().unwrap().to_owned());

    let stored = devices::Entity::find().one(&app.state.db).await.unwrap();
    assert!(stored.unwrap().expires_at.is_none());
    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);
    let resp = app.get("/api/v1/devices", &user).await;
    assert!(resp.body[0]["expires_at"].is_null());

    // Disabled accounts are still refused.
    let user_id = app.user_id(&user).await;
    let patch = |disabled: bool| {
        app.call(
            "PATCH",
            &format!("/api/v1/admin/users/{user_id}"),
            &admin,
            Some(json!({ "disabled": disabled })),
        )
    };
    patch(true).await.assert(StatusCode::OK);
    app.get("/api/v1/me", &bearer)
        .await
        .assert_error(StatusCode::FORBIDDEN, "account_disabled");
    patch(false).await.assert(StatusCode::OK);
    app.get("/api/v1/me", &bearer).await.assert(StatusCode::OK);

    // Revoking still works (disabling signed the web session out: sign in again).
    let user = As::Cookie(
        app.login("user@example.com", super::support::PASSWORD)
            .await
            .cookie()
            .unwrap(),
    );
    app.call("DELETE", &format!("/api/v1/devices/{id}"), &user, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    app.get("/api/v1/me", &bearer)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn invites_refuse_the_forever_lifetime() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    app.post(
        "/api/v1/admin/invites",
        &admin,
        json!({ "expires_in": "forever" }),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
}
