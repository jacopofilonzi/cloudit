//! Registration, login/logout, sessions, CSRF, password reset.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cloudit_core::protocol::{NewAccounts, RegistrationMode};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::json;

use super::support::{ADMIN_EMAIL, As, PASSWORD, TestApp};
use crate::db::entities::sessions;

fn register_body(email: &str, invite: Option<&str>) -> serde_json::Value {
    let mut body = json!({ "email": email, "password": PASSWORD, "display_name": "User" });
    if let Some(invite) = invite {
        body["invite_token"] = json!(invite);
    }
    body
}

async fn invite(app: &TestApp, admin: &As, email: Option<&str>) -> String {
    let body = match email {
        Some(email) => json!({ "email": email }),
        None => json!({}),
    };
    let resp = app.post("/api/v1/admin/invites", admin, body).await;
    resp.assert(StatusCode::CREATED);
    resp.body["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn register_in_open_mode() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    app.set_mode(RegistrationMode::Open).await;
    app.set_password_policy(NewAccounts::Auto).await;
    let resp = app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("New@Example.com", None),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email"], "new@example.com");
    assert_eq!(resp.body["email_verified"], false);
    assert_eq!(resp.body["is_admin"], false);
    assert_eq!(resp.body["has_password"], true);
    let cookie = As::Cookie(resp.cookie().unwrap());
    app.get("/api/v1/me", &cookie).await.assert(StatusCode::OK);

    // Same email (any case): taken.
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("NEW@example.com", None),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "email_taken");
    // Validation.
    let short = json!({ "email": "x@example.com", "password": "short", "display_name": "X" });
    let resp = app.post("/api/v1/auth/register", &As::Anon, short).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "password");
    // Unknown fields are rejected.
    let extra = json!({ "email": "y@example.com", "password": PASSWORD, "display_name": "Y", "admin": true });
    app.post("/api/v1/auth/register", &As::Anon, extra)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
}

#[tokio::test]
async fn register_in_invite_mode_needs_a_valid_invite() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    // Default mode after setup is invite.
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("a@example.com", None),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("a@example.com", Some("made-up")),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");

    // An open invite works once.
    let token = invite(&app, &admin, None).await;
    let resp = app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("a@example.com", Some(&token)),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email_verified"], false);
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("b@example.com", Some(&token)),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");

    // An invite bound to an email: only that email, which becomes verified.
    let token = invite(&app, &admin, Some("Bound@Example.com")).await;
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("other@example.com", Some(&token)),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    let resp = app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("bound@example.com", Some(&token)),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email_verified"], true);

    // A failed registration (email taken) doesn't consume the invite.
    let token = invite(&app, &admin, None).await;
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body(ADMIN_EMAIL, Some(&token)),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "email_taken");
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("c@example.com", Some(&token)),
    )
    .await
    .assert(StatusCode::CREATED);

    // Revoked invites don't work.
    let resp = app.post("/api/v1/admin/invites", &admin, json!({})).await;
    let (id, token) = (
        resp.body["id"].as_str().unwrap().to_owned(),
        resp.body["token"].as_str().unwrap().to_owned(),
    );
    app.call(
        "DELETE",
        &format!("/api/v1/admin/invites/{id}"),
        &admin,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("d@example.com", Some(&token)),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
}

#[tokio::test]
async fn expired_invite_is_refused() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let token = invite(&app, &admin, None).await;
    crate::db::entities::invites::Entity::update_many()
        .col_expr(
            crate::db::entities::invites::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&app.state.db)
        .await
        .unwrap();
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("a@example.com", Some(&token)),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
}

#[tokio::test]
async fn login_and_logout() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    app.user("user@example.com").await;

    app.login("user@example.com", "wrong password")
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    app.login("nobody@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    app.login("not an email", PASSWORD)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");

    let resp = app.login(" USER@example.com", PASSWORD).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["email"], "user@example.com");
    let set_cookie = resp.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(set_cookie.contains("SameSite=Lax"), "{set_cookie}");
    assert!(set_cookie.contains("Path=/"), "{set_cookie}");
    assert!(!set_cookie.contains("Secure"), "plain http: {set_cookie}");
    let cookie = As::Cookie(resp.cookie().unwrap());
    app.get("/api/v1/me", &cookie).await.assert(StatusCode::OK);

    let resp = app.post("/api/v1/auth/logout", &cookie, json!({})).await;
    resp.assert(StatusCode::NO_CONTENT);
    assert_eq!(resp.cookie().as_deref(), Some(""));
    app.get("/api/v1/me", &cookie)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    // Logging out without a session is fine.
    app.post("/api/v1/auth/logout", &As::Anon, json!({}))
        .await
        .assert(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn secure_cookie_with_https_public_url() {
    let app = TestApp::with(&[("PUBLIC_URL", "https://localhost")]).await;
    let token = app.setup_token().await;
    let request = Request::post("/api/v1/setup")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "https://localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "token": token, "email": ADMIN_EMAIL, "password": PASSWORD, "display_name": "A" })
                .to_string(),
        ))
        .unwrap();
    let resp = app.send(request).await;
    resp.assert(StatusCode::CREATED);
    assert!(
        resp.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .ends_with("; Secure")
    );
}

#[tokio::test]
async fn disabled_user_is_locked_out() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let id = app.user_id(&user).await;

    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{id}"),
        &admin,
        Some(json!({ "disabled": true })),
    )
    .await
    .assert(StatusCode::OK);
    // The sessions are deleted at once; login is refused after the password check.
    app.get("/api/v1/me", &user)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    app.login("user@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::FORBIDDEN, "account_disabled");
    app.login("user@example.com", "wrong password")
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");

    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{id}"),
        &admin,
        Some(json!({ "disabled": false })),
    )
    .await
    .assert(StatusCode::OK);
    // Re-enabling doesn't revive the old session; signing in again works.
    app.get("/api/v1/me", &user)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    app.login("user@example.com", PASSWORD)
        .await
        .assert(StatusCode::OK);
}

#[tokio::test]
async fn login_is_rate_limited_per_email() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    for _ in 0..10 {
        app.login(ADMIN_EMAIL, "wrong password")
            .await
            .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    }
    let resp = app.login(ADMIN_EMAIL, PASSWORD).await;
    resp.assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    let retry: u64 = resp.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=900).contains(&retry), "{retry}");
    // Other accounts are limited per IP only (20 per window).
    app.login("other@example.com", "x")
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
}

async fn session_row(app: &TestApp, token: &str) -> sessions::Model {
    sessions::Entity::find()
        .filter(sessions::Column::TokenHash.eq(crate::auth::tokens::hash_token(token)))
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn sessions_slide_at_most_once_per_hour() {
    let app = TestApp::new().await;
    let As::Cookie(token) = app.setup_admin().await else {
        unreachable!()
    };
    let cookie = As::Cookie(token.clone());
    let before = session_row(&app, &token).await;

    // A fresh session: no write, no new cookie.
    let resp = app.get("/api/v1/me", &cookie).await;
    resp.assert(StatusCode::OK);
    assert!(resp.cookie().is_none());
    assert_eq!(session_row(&app, &token).await, before);

    // Last seen two hours ago: extended, cookie refreshed.
    let two_hours_ago = crate::time::now() - chrono::TimeDelta::hours(2);
    sessions::Entity::update_many()
        .col_expr(sessions::Column::LastSeenAt, Expr::value(two_hours_ago))
        .col_expr(
            sessions::Column::ExpiresAt,
            Expr::value(two_hours_ago + chrono::TimeDelta::days(30)),
        )
        .filter(sessions::Column::Id.eq(before.id))
        .exec(&app.state.db)
        .await
        .unwrap();
    let resp = app.get("/api/v1/me", &cookie).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.cookie().as_deref(), Some(token.as_str()));
    let after = session_row(&app, &token).await;
    assert!(after.last_seen_at > two_hours_ago + chrono::TimeDelta::minutes(90));
    assert!(after.expires_at > crate::time::now() + chrono::TimeDelta::days(29));

    // Expired: refused.
    sessions::Entity::update_many()
        .col_expr(
            sessions::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .filter(sessions::Column::Id.eq(before.id))
        .exec(&app.state.db)
        .await
        .unwrap();
    app.get("/api/v1/me", &cookie)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn csrf_rules() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let As::Cookie(token) = &admin else {
        unreachable!()
    };
    let patch = |origin: Option<&str>, fetch_site: Option<&str>| {
        let mut builder = Request::patch("/api/v1/me")
            .header(header::HOST, "localhost")
            .header(header::COOKIE, format!("cloudit_session={token}"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(site) = fetch_site {
            builder = builder.header("sec-fetch-site", site);
        }
        builder
            .body(Body::from(json!({ "display_name": "Changed" }).to_string()))
            .unwrap()
    };

    app.send(patch(Some("http://localhost"), None))
        .await
        .assert(StatusCode::OK);
    app.send(patch(None, Some("same-origin")))
        .await
        .assert(StatusCode::OK);
    app.send(patch(Some("http://evil.example"), None))
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");
    app.send(patch(Some("null"), None))
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");
    app.send(patch(None, Some("cross-site")))
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");
    app.send(patch(None, None))
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");

    // Login from another site is refused too (login CSRF).
    let login = Request::post("/api/v1/auth/login")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "email": ADMIN_EMAIL, "password": PASSWORD }).to_string(),
        ))
        .unwrap();
    app.send(login)
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");

    // Safe methods are not checked.
    let get = Request::get("/api/v1/me")
        .header(header::HOST, "localhost")
        .header(header::COOKIE, format!("cloudit_session={token}"))
        .body(Body::empty())
        .unwrap();
    app.send(get).await.assert(StatusCode::OK);
}

#[tokio::test]
async fn csrf_uses_public_url_and_trusted_proxy_headers() {
    let app = TestApp::with(&[("PUBLIC_URL", "https://sync.example.com")]).await;
    let token = app.setup_token().await;
    let body =
        json!({ "token": token, "email": ADMIN_EMAIL, "password": PASSWORD, "display_name": "A" });
    // The Host-derived origin no longer counts.
    let request = Request::post("/api/v1/setup")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.send(request)
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");

    let app = TestApp::with(&[("TRUST_PROXY_HEADERS", "true")]).await;
    let token = app.setup_token().await;
    let body =
        json!({ "token": token, "email": ADMIN_EMAIL, "password": PASSWORD, "display_name": "A" });
    let request = Request::post("/api/v1/setup")
        .header(header::HOST, "cloudit-1:8080")
        .header("x-forwarded-host", "sync.example.com")
        .header("x-forwarded-proto", "https")
        .header(header::ORIGIN, "https://sync.example.com")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.send(request).await;
    resp.assert(StatusCode::CREATED);
    assert!(
        resp.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
}

#[tokio::test]
async fn password_reset_sets_the_password_and_signs_out() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let other_session = As::Cookie(
        app.login("user@example.com", PASSWORD)
            .await
            .cookie()
            .unwrap(),
    );
    let id = app.user_id(&user).await;

    let resp = app
        .post(
            &format!("/api/v1/admin/users/{id}/password-reset"),
            &admin,
            json!({}),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    let token = resp.body["token"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["path"], format!("/reset-password/{token}"));

    let reset = |token: &str, password: &str| {
        let body = json!({ "token": token, "new_password": password });
        app.post("/api/v1/auth/password-reset", &As::Anon, body)
    };
    reset(&token, "short")
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    reset("made-up", "a brand new password")
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
    let resp = reset(&token, "a brand new password").await;
    resp.assert(StatusCode::NO_CONTENT);
    assert!(resp.cookie().is_none(), "no automatic sign-in");
    reset(&token, "yet another password")
        .await
        .assert_error(StatusCode::BAD_REQUEST, "token_invalid");

    // Every session of the account is gone; the new password works, the old doesn't.
    for session in [&user, &other_session] {
        app.get("/api/v1/me", session)
            .await
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    app.login("user@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    app.login("user@example.com", "a brand new password")
        .await
        .assert(StatusCode::OK);
    // The admin's session is untouched.
    app.get("/api/v1/me", &admin).await.assert(StatusCode::OK);
}

#[tokio::test]
async fn a_new_password_reset_invalidates_older_ones() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let user = app.user("user@example.com").await;
    let id = app.user_id(&user).await;
    let admin_id = app.user_id(&admin).await;
    let create = |id: String| {
        let app = &app;
        let admin = &admin;
        async move {
            let resp = app
                .post(
                    &format!("/api/v1/admin/users/{id}/password-reset"),
                    admin,
                    json!({}),
                )
                .await;
            resp.assert(StatusCode::CREATED);
            resp.body["token"].as_str().unwrap().to_owned()
        }
    };
    let old = create(id.clone()).await;
    let admin_reset = create(admin_id).await;
    // The CLI creates resets too.
    let cli = crate::cli::reset_password_text(&app.cli_config(), "user@example.com")
        .await
        .unwrap();
    let from_cli = cli
        .rsplit("/reset-password/")
        .next()
        .unwrap()
        .trim()
        .to_owned();
    let newest = create(id).await;
    let reset = |token: String| {
        let body = json!({ "token": token, "new_password": "a brand new password" });
        app.post("/api/v1/auth/password-reset", &As::Anon, body)
    };
    for stale in [old, from_cli] {
        reset(stale)
            .await
            .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
    }
    // Another user's reset is unaffected.
    reset(admin_reset).await.assert(StatusCode::NO_CONTENT);
    reset(newest).await.assert(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn expired_password_reset_is_refused() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let id = app.user_id(&admin).await;
    let resp = app
        .post(
            &format!("/api/v1/admin/users/{id}/password-reset"),
            &admin,
            json!({}),
        )
        .await;
    let token = resp.body["token"].as_str().unwrap().to_owned();
    crate::db::entities::password_resets::Entity::update_many()
        .col_expr(
            crate::db::entities::password_resets::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&app.state.db)
        .await
        .unwrap();
    app.post(
        "/api/v1/auth/password-reset",
        &As::Anon,
        json!({ "token": token, "new_password": "a brand new password" }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "token_invalid");
}

#[tokio::test]
async fn cli_reset_password_works_for_any_account() {
    let app = TestApp::with(&[("PUBLIC_URL", "https://sync.example.com")]).await;
    app.setup_admin().await;
    let text = crate::cli::reset_password_text(&app.cli_config(), "Admin@Example.com")
        .await
        .unwrap();
    let url_line = text.lines().find(|l| l.contains("url:")).unwrap();
    let token = url_line
        .rsplit("/reset-password/")
        .next()
        .unwrap()
        .to_owned();
    assert!(
        url_line.contains("https://sync.example.com/reset-password/"),
        "{text}"
    );
    let reset = crate::db::entities::password_resets::Entity::find()
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reset.created_by, None);
    assert!(!text.contains("hash"));

    // Make the account password-less (as an OIDC-only account would be): the reset sets one.
    crate::db::entities::users::Entity::update_many()
        .col_expr(
            crate::db::entities::users::Column::PasswordHash,
            Expr::value(Option::<String>::None),
        )
        .exec(&app.state.db)
        .await
        .unwrap();
    // Requests from here on need a valid Origin (setup is done, PUBLIC_URL is https).
    let request = Request::post("/api/v1/auth/password-reset")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "https://sync.example.com")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "token": token, "new_password": "a brand new password" }).to_string(),
        ))
        .unwrap();
    app.send(request).await.assert(StatusCode::NO_CONTENT);
    let user = crate::accounts::users::find_by_email(&app.state.db, ADMIN_EMAIL)
        .await
        .unwrap()
        .unwrap();
    assert!(user.password_hash.is_some());

    assert!(
        crate::cli::reset_password_text(&app.cli_config(), "nobody@example.com")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invite_lookup() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let lookup = |token: String| {
        app.post(
            "/api/v1/auth/invites/lookup",
            &As::Anon,
            json!({ "token": token }),
        )
    };

    let token = invite(&app, &admin, Some("Bound@Example.com")).await;
    let resp = lookup(token.clone()).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["email"], "bound@example.com");
    assert_eq!(resp.body["invited_by"], "Admin");
    assert!(resp.body["expires_at"].as_str().unwrap().ends_with('Z'));

    let open = invite(&app, &admin, None).await;
    assert_eq!(lookup(open.clone()).await.body["email"], json!(null));

    // Unknown and used invites look the same.
    lookup("nope".to_owned())
        .await
        .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("bound@example.com", Some(&token)),
    )
    .await
    .assert(StatusCode::CREATED);
    lookup(token)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    app.post(
        "/api/v1/auth/invites/lookup",
        &As::Anon,
        json!({ "token": open, "extra": 1 }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
}
