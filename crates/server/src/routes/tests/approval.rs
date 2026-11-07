//! Admin approval of new accounts: password registration and provider onboarding
//! (`auto` / `approval`, invites, admin groups), sign-in of pending accounts.

use axum::http::StatusCode;
use cloudit_core::protocol::{NewAccounts, RegistrationMode};
use serde_json::{Value, json};

use super::fake_idp::IdClaims;
use super::oauth::{Env, assert_login_error, assert_signed_in};
use super::onboarding::{create, pending, with_cookie};
use super::support::{As, PASSWORD, Resp, TestApp};

fn register_body(email: &str) -> Value {
    json!({ "email": email, "password": PASSWORD, "display_name": "User" })
}

/// A 202 `{status: "pending"}` without a session.
#[track_caller]
fn assert_pending_signup(resp: &Resp) {
    resp.assert(StatusCode::ACCEPTED);
    assert_eq!(resp.body, json!({ "status": "pending" }));
    assert!(resp.cookie().is_none(), "no session for a pending account");
}

async fn admin_view(app: &TestApp, admin: &As, email: &str) -> Value {
    let resp = app.get("/api/v1/admin/users?limit=200", admin).await;
    resp.assert(StatusCode::OK);
    resp.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == email)
        .cloned()
        .unwrap_or_else(|| panic!("no user {email}"))
}

async fn invite(app: &TestApp, admin: &As) -> String {
    let resp = app.post("/api/v1/admin/invites", admin, json!({})).await;
    resp.assert(StatusCode::CREATED);
    resp.body["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn password_registration_follows_the_setting() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    app.set_mode(RegistrationMode::Open).await;
    let config = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(config.body["password_requires_approval"], true, "default");

    // Default `approval`: created pending, no session.
    let resp = app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("p@example.com"),
        )
        .await;
    assert_pending_signup(&resp);
    let user = admin_view(&app, &admin, "p@example.com").await;
    assert_eq!(user["approval_status"], "pending");
    assert_eq!(user["signup_source"], "password");
    assert_eq!(user["approval_decided_at"], Value::Null);
    // Login: a wrong password still says only `invalid_credentials`; the state is
    // revealed only after the password is verified.
    app.login("p@example.com", "wrong password")
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    app.login("p@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::FORBIDDEN, "account_pending");
    // The email stays taken.
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        register_body("p@example.com"),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "email_taken");

    // `auto`: active at once.
    app.set_password_policy(NewAccounts::Auto).await;
    let config = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(config.body["password_requires_approval"], false);
    let resp = app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("a@example.com"),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    let session = As::Cookie(resp.cookie().unwrap());
    app.get("/api/v1/me", &session).await.assert(StatusCode::OK);
    let user = admin_view(&app, &admin, "a@example.com").await;
    assert_eq!(user["approval_status"], "approved");
    assert_eq!(user["signup_source"], "password");
    // The first admin: approved, no source.
    let first = admin_view(&app, &admin, super::support::ADMIN_EMAIL).await;
    assert_eq!(first["approval_status"], "approved");
    assert_eq!(first["signup_source"], Value::Null);
}

#[tokio::test]
async fn an_invite_skips_password_approval() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    for mode in [RegistrationMode::Invite, RegistrationMode::Open] {
        app.set_mode(mode).await;
        let token = invite(&app, &admin).await;
        let email = format!("{mode:?}@example.com").to_lowercase();
        let mut body = register_body(&email);
        body["invite_token"] = json!(token);
        let resp = app.post("/api/v1/auth/register", &As::Anon, body).await;
        resp.assert(StatusCode::CREATED);
        assert!(resp.cookie().is_some());
        let user = admin_view(&app, &admin, &email).await;
        assert_eq!(user["approval_status"], "approved");
        assert_eq!(user["signup_source"], "invite");
    }
}

/// Switches provider `id` to `approval`.
async fn require_approval(env: &Env, id: &str) {
    env.app
        .call(
            "PATCH",
            &format!("/api/v1/admin/providers/{id}"),
            &env.admin,
            Some(json!({ "new_accounts": "approval" })),
        )
        .await
        .assert(StatusCode::OK);
}

#[tokio::test]
async fn provider_approval_creates_pending_accounts() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    require_approval(&env, "gh").await;

    let resp = env
        .github(7, Some("octo@example.com"), "login", &As::Anon)
        .await;
    assert_eq!(resp.location(), "/onboarding");
    let token = resp.cookie_named("cloudit_onboarding").unwrap();
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["requires_approval"], true);
    let resp = create(&env, &token, json!({ "display_name": "Octo" })).await;
    assert_pending_signup(&resp);
    assert_eq!(
        resp.cookie_named("cloudit_onboarding").as_deref(),
        Some(""),
        "onboarding cookie cleared"
    );
    let user = admin_view(&env.app, &env.admin, "octo@example.com").await;
    assert_eq!(user["approval_status"], "pending");
    assert_eq!(user["signup_source"], "gh");
    assert_eq!(user["identity_providers"], json!(["gh"]));

    // Signing in again with the identity: pending, and no second account.
    assert_login_error(
        &env.github(7, Some("octo@example.com"), "login", &As::Anon)
            .await,
        "account_pending",
    );
    let list = env.app.get("/api/v1/admin/users", &env.admin).await;
    assert_eq!(list.body["total"], 2);

    // `auto` (the other providers of the test env): active at once.
    let resp = env
        .discord("9", "wump@example.com", true, "login", &As::Anon)
        .await;
    let token = resp.cookie_named("cloudit_onboarding").unwrap();
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["requires_approval"], false);
    let resp = create(&env, &token, json!({ "display_name": "W" })).await;
    resp.assert(StatusCode::CREATED);
    assert!(resp.cookie().is_some());
    let user = admin_view(&env.app, &env.admin, "wump@example.com").await;
    assert_eq!(user["approval_status"], "approved");
    assert_eq!(user["signup_source"], "dc");
}

#[tokio::test]
async fn invites_and_admin_groups_skip_provider_approval() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    require_approval(&env, "sso").await;

    // In the provider's admin groups: approved (the provider vouches for it).
    let token = pending(
        &env,
        IdClaims::new("boss")
            .email("boss@example.com", true)
            .groups(&["admins"]),
    )
    .await;
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["requires_approval"], false);
    let resp = create(&env, &token, json!({ "display_name": "Boss" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["is_admin"], true);

    // Not in them: pending, unless an invite is given.
    let token = pending(&env, IdClaims::new("u").email("u@example.com", true)).await;
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["requires_approval"], true);
    let token_u = invite(&env.app, &env.admin).await;
    let resp = create(
        &env,
        &token,
        json!({ "display_name": "U", "invite_token": token_u }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    let user = admin_view(&env.app, &env.admin, "u@example.com").await;
    assert_eq!(user["approval_status"], "approved");
    assert_eq!(user["signup_source"], "invite");
    assert_signed_in(
        &env.oidc(IdClaims::new("u"), "login", &As::Anon).await,
        "/after",
    );

    // An invite carried from the sign-in start counts too (and shows in the info).
    let carried = invite(&env.app, &env.admin).await;
    let started = Env::started(
        &env.start_with(
            "sso",
            json!({ "purpose": "login", "invite_token": carried }),
            &As::Anon,
        )
        .await,
    );
    let mut claims = IdClaims::new("c").email("c@example.com", true);
    claims.nonce.clone_from(&started.nonce);
    env.idp.default_userinfo("oidc-c", json!({ "sub": "c" }));
    let resp = env
        .finish(
            "sso",
            &started,
            Some(env.idp.id_token(&claims)),
            "oidc-c",
            &As::Anon,
        )
        .await;
    let token = resp.cookie_named("cloudit_onboarding").unwrap();
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["requires_approval"], false);
    create(&env, &token, json!({ "display_name": "C" }))
        .await
        .assert(StatusCode::CREATED);
}

#[tokio::test]
async fn pending_accounts_cant_be_linked_into() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    // A pending password account…
    env.app
        .post(
            "/api/v1/auth/register",
            &As::Anon,
            register_body("p@example.com"),
        )
        .await
        .assert(StatusCode::ACCEPTED);
    // …can't be proven at onboarding (after the password check)…
    let token = pending(&env, IdClaims::new("x").email("x@example.com", true)).await;
    with_cookie(
        &env,
        "POST",
        "/api/v1/auth/onboarding/link",
        &token,
        Some(json!({ "email": "p@example.com", "password": "nope nope nope" })),
    )
    .await
    .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    with_cookie(
        &env,
        "POST",
        "/api/v1/auth/onboarding/link",
        &token,
        Some(json!({ "email": "p@example.com", "password": PASSWORD })),
    )
    .await
    .assert_error(StatusCode::FORBIDDEN, "account_pending");
    assert_eq!(env.identity_rows().await.len(), 0, "nothing linked");
}
