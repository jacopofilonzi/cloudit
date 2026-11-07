//! Onboarding of unknown identities (create an account, link with a password or
//! another provider), `/me/identities` and the admin user list's identities.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cloudit_core::protocol::RegistrationMode;
use sea_orm::EntityTrait as _;
use sea_orm::sea_query::Expr;
use serde_json::{Value, json};

use super::fake_idp::IdClaims;
use super::oauth::{Env, assert_login_error, assert_signed_in};
use super::support::{ADMIN_EMAIL, As, PASSWORD, Resp};
use crate::db::entities::pending_signups;

/// Sends a request carrying the onboarding cookie.
/// Builds the request now, so the future borrows only `env`.
fn with_cookie<'a>(
    env: &'a Env,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> impl Future<Output = Resp> + use<'a> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, &env.app.origin)
        .header(header::COOKIE, format!("cloudit_onboarding={token}"));
    let body = match body {
        Some(b) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    env.app.send(builder.body(body).unwrap())
}

/// Signs in an unknown OIDC identity; returns the onboarding token.
async fn pending(env: &Env, claims: IdClaims) -> String {
    let resp = env.oidc(claims, "login", &As::Anon).await;
    assert_eq!(resp.location(), "/onboarding", "{}", resp.body);
    resp.cookie_named("cloudit_onboarding").unwrap()
}

async fn create(env: &Env, token: &str, body: Value) -> Resp {
    with_cookie(
        env,
        "POST",
        "/api/v1/auth/onboarding/create",
        token,
        Some(body),
    )
    .await
}

#[track_caller]
fn assert_cleared_onboarding(resp: &Resp) {
    assert_eq!(resp.cookie_named("cloudit_onboarding").as_deref(), Some(""));
}

#[tokio::test]
async fn create_with_the_providers_email() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    let token = pending(
        &env,
        IdClaims::new("n1")
            .email("New@Example.com", true)
            .name("New Person")
            .groups(&["admins"]),
    )
    .await;
    let resp = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    resp.assert(StatusCode::OK);
    assert_eq!(
        resp.body,
        json!({
            "provider": { "id": "sso", "kind": "oidc", "display_name": "OpenID Connect", "icon": "generic" },
            "email": "new@example.com", "email_from_provider": true, "email_verified": true,
            "display_name": "New Person", "registration_mode": "open", "invite_required": false,
            "requires_approval": false
        })
    );
    let resp = create(&env, &token, json!({ "display_name": "Newbie" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email"], "new@example.com");
    assert_eq!(resp.body["email_verified"], true);
    assert_eq!(resp.body["display_name"], "Newbie");
    assert_eq!(resp.body["has_password"], false);
    assert_eq!(resp.body["is_admin"], true, "in the admin group");
    assert_cleared_onboarding(&resp);
    let session = As::Cookie(resp.cookie().unwrap());
    env.app
        .get("/api/v1/me", &session)
        .await
        .assert(StatusCode::OK);
    // Single use.
    with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None)
        .await
        .assert_error(StatusCode::GONE, "onboarding_expired");
    create(&env, &token, json!({ "display_name": "Again" }))
        .await
        .assert_error(StatusCode::GONE, "onboarding_expired");
    // The identity now signs in.
    let session = assert_signed_in(
        &env.oidc(IdClaims::new("n1"), "login", &As::Anon).await,
        "/after",
    );
    assert_eq!(
        env.app.user_id(&session).await,
        resp.body["id"].as_str().unwrap()
    );
}

#[tokio::test]
async fn create_email_rules() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    // Another email than the provider's: unverified; password optional but checked.
    let token = pending(&env, IdClaims::new("a").email("a@example.com", true)).await;
    let resp = create(
        &env,
        &token,
        json!({ "display_name": "A", "email": "other@example.com", "password": "short" }),
    )
    .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "password");
    let resp = create(
        &env,
        &token,
        json!({ "display_name": "A", "email": "Other@Example.com", "password": PASSWORD }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email"], "other@example.com");
    assert_eq!(resp.body["email_verified"], false);
    assert_eq!(resp.body["has_password"], true);
    env.app
        .login("other@example.com", PASSWORD)
        .await
        .assert(StatusCode::OK);

    // The provider's email, unverified by the provider: kept, unverified.
    let token = pending(&env, IdClaims::new("b").email("b@example.com", false)).await;
    let resp = create(&env, &token, json!({ "display_name": "B" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email_verified"], false);

    // No email from the provider: required.
    let token = pending(&env, IdClaims::new("c")).await;
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["email"], Value::Null);
    assert_eq!(info.body["email_from_provider"], false);
    let resp = create(&env, &token, json!({ "display_name": "C" })).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "email");
    // Taken email: refused, the pending signup survives (the user can link instead).
    create(
        &env,
        &token,
        json!({ "display_name": "C", "email": ADMIN_EMAIL }),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "email_taken");
    let resp = create(
        &env,
        &token,
        json!({ "display_name": "C", "email": "c@example.com" }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email_verified"], false);
}

#[tokio::test]
async fn create_honours_registration_mode_and_invites() {
    let env = Env::new().await;
    // Closed: no account (linking stays possible).
    env.app.set_mode(RegistrationMode::Closed).await;
    let token = pending(&env, IdClaims::new("a").email("a@example.com", true)).await;
    create(&env, &token, json!({ "display_name": "A" }))
        .await
        .assert_error(StatusCode::FORBIDDEN, "registration_closed");

    // Invite mode: an invite is required and consumed.
    env.app.set_mode(RegistrationMode::Invite).await;
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["invite_required"], true);
    create(&env, &token, json!({ "display_name": "A" }))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    let invite = |email: Option<&str>| {
        let body = match email {
            Some(e) => json!({ "email": e }),
            None => json!({}),
        };
        env.app.post("/api/v1/admin/invites", &env.admin, body)
    };
    let bound = invite(Some("someone@example.com")).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    create(
        &env,
        &token,
        json!({ "display_name": "A", "invite_token": bound }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "invite_invalid");
    let open = invite(None).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    create(
        &env,
        &token,
        json!({ "display_name": "A", "invite_token": open.clone() }),
    )
    .await
    .assert(StatusCode::CREATED);
    let resp = env.app.get("/api/v1/admin/invites", &env.admin).await;
    assert!(
        resp.body
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["status"] == "used")
    );

    // An invite carried from the sign-in start: not required any more; bound to the
    // provider's email it makes even an unverified email verified.
    let bound = invite(Some("b@example.com")).await.body["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = env
        .start_with(
            "sso",
            json!({ "purpose": "login", "invite_token": bound }),
            &As::Anon,
        )
        .await;
    let started = Env::started(&started);
    let mut claims = IdClaims::new("b").email("b@example.com", false);
    claims.nonce.clone_from(&started.nonce);
    env.idp.default_userinfo("oidc-b", json!({ "sub": "b" }));
    let resp = env
        .finish(
            "sso",
            &started,
            Some(env.idp.id_token(&claims)),
            "oidc-b",
            &As::Anon,
        )
        .await;
    let token = resp.cookie_named("cloudit_onboarding").unwrap();
    let info = with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None).await;
    assert_eq!(info.body["invite_required"], false);
    let resp = create(&env, &token, json!({ "display_name": "B" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["email_verified"], true);
}

#[tokio::test]
async fn link_with_a_password() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    let user_id = env.app.user_id(&user).await;
    let token = pending(
        &env,
        IdClaims::new("x")
            .email("x@example.com", true)
            .groups(&["admins"]),
    )
    .await;
    let link = |email: &str, password: &str| {
        with_cookie(
            &env,
            "POST",
            "/api/v1/auth/onboarding/link",
            &token,
            Some(json!({ "email": email, "password": password })),
        )
    };
    link("user@example.com", "wrong password")
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    link("nobody@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "invalid_credentials");
    let resp = link("User@Example.com", PASSWORD).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["id"], user_id);
    assert_eq!(
        resp.body["is_admin"], true,
        "groups of the linked OIDC identity"
    );
    assert_cleared_onboarding(&resp);
    link("user@example.com", PASSWORD)
        .await
        .assert_error(StatusCode::GONE, "onboarding_expired");
    let session = assert_signed_in(
        &env.oidc(IdClaims::new("x"), "login", &As::Anon).await,
        "/after",
    );
    assert_eq!(env.app.user_id(&session).await, user_id);
}

#[tokio::test]
async fn link_with_another_provider() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    let user_id = env.app.user_id(&user).await;
    env.github(77, None, "link", &user).await;

    let token = pending(&env, IdClaims::new("new-oidc").email("x@example.com", true)).await;
    let start = |provider: &str| {
        with_cookie(
            &env,
            "POST",
            &format!("/api/v1/auth/onboarding/link/{provider}/start"),
            &token,
            None,
        )
    };
    // An identity no account owns can't prove anything.
    env.idp
        .set_github_user("gh-78", json!({ "id": 78, "login": "x" }), json!([]));
    let started = Env::started(&start("gh").await);
    assert_login_error(
        &env.finish("gh", &started, None, "gh-78", &As::Anon).await,
        "identity_not_linked",
    );

    // The linked GitHub account: the OIDC identity is attached and the user signed in.
    let started = Env::started(&start("gh").await);
    let resp = env.finish("gh", &started, None, "gh-77", &As::Anon).await;
    let session = assert_signed_in(&resp, "/");
    assert_cleared_onboarding(&resp);
    assert_eq!(env.app.user_id(&session).await, user_id);
    let session = assert_signed_in(
        &env.oidc(IdClaims::new("new-oidc"), "login", &As::Anon)
            .await,
        "/after",
    );
    assert_eq!(env.app.user_id(&session).await, user_id);
    // The pending signup is used up.
    start("gh")
        .await
        .assert_error(StatusCode::GONE, "onboarding_expired");

    // The pending identity got linked meanwhile (here: to the admin): identity_in_use.
    let token = pending(&env, IdClaims::new("raced")).await;
    let resp = with_cookie(
        &env,
        "POST",
        "/api/v1/auth/onboarding/link/gh/start",
        &token,
        None,
    )
    .await;
    let started = Env::started(&resp);
    env.oidc(IdClaims::new("raced"), "link", &env.admin).await;
    assert_login_error(
        &env.finish("gh", &started, None, "gh-77", &As::Anon).await,
        "identity_in_use",
    );
}

#[tokio::test]
async fn onboarding_needs_a_live_pending_signup() {
    let env = Env::new().await;
    for (method, uri, body) in [
        ("GET", "/api/v1/auth/onboarding", None),
        (
            "POST",
            "/api/v1/auth/onboarding/create",
            Some(json!({ "display_name": "A" })),
        ),
        (
            "POST",
            "/api/v1/auth/onboarding/link",
            Some(json!({ "email": "a@b.co", "password": "x" })),
        ),
        ("POST", "/api/v1/auth/onboarding/link/gh/start", None),
    ] {
        env.app
            .call(method, uri, &As::Anon, body.clone())
            .await
            .assert_error(StatusCode::GONE, "onboarding_expired");
        with_cookie(&env, method, uri, &"z".repeat(43), body)
            .await
            .assert_error(StatusCode::GONE, "onboarding_expired");
    }
    let token = pending(&env, IdClaims::new("late").email("late@example.com", true)).await;
    pending_signups::Entity::update_many()
        .col_expr(
            pending_signups::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&env.app.state.db)
        .await
        .unwrap();
    with_cookie(&env, "GET", "/api/v1/auth/onboarding", &token, None)
        .await
        .assert_error(StatusCode::GONE, "onboarding_expired");
}

#[tokio::test]
async fn my_identities_list_and_unlink() {
    let env = Env::new().await;
    env.app.set_mode(RegistrationMode::Open).await;
    // A provider-only account with two identities.
    let token = pending(&env, IdClaims::new("p").email("p@example.com", true)).await;
    let session = As::Cookie(
        create(&env, &token, json!({ "display_name": "P" }))
            .await
            .cookie()
            .unwrap(),
    );
    env.github(5, Some("p@example.com"), "link", &session).await;
    let resp = env.app.get("/api/v1/me/identities", &session).await;
    resp.assert(StatusCode::OK);
    let list = resp.body.as_array().unwrap().clone();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["provider"]["id"], "sso");
    assert_eq!(list[0]["provider"]["kind"], "oidc");
    assert_eq!(list[0]["email"], "p@example.com");
    assert!(list[0]["last_login_at"].is_string());
    assert_eq!(list[1]["provider"]["id"], "gh");
    assert_eq!(list[1]["last_login_at"], Value::Null);
    let id = |i: usize| list[i]["id"].as_str().unwrap().to_owned();

    // Admins see the providers.
    let resp = env.app.get("/api/v1/admin/users", &env.admin).await;
    let me = resp.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "p@example.com")
        .unwrap()
        .clone();
    assert_eq!(me["identity_providers"], json!(["gh", "sso"]));
    assert_eq!(me["has_password"], false);

    // GitHub disabled: it doesn't count as a way in, so the OIDC one is the last.
    env.app
        .call(
            "PATCH",
            "/api/v1/admin/providers/gh",
            &env.admin,
            Some(json!({ "enabled": false })),
        )
        .await
        .assert(StatusCode::OK);
    let unlink = |id: String| {
        env.app.call(
            "DELETE",
            &format!("/api/v1/me/identities/{id}"),
            &session,
            None,
        )
    };
    unlink(id(0))
        .await
        .assert_error(StatusCode::CONFLICT, "last_login_method");
    // Deleted providers are flagged.
    env.app
        .call("DELETE", "/api/v1/admin/providers/gh", &env.admin, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    let resp = env.app.get("/api/v1/me/identities", &session).await;
    assert_eq!(resp.body[1]["provider"]["deleted"], true);
    assert_eq!(resp.body[1]["provider"]["enabled"], false);
    assert_eq!(resp.body[0]["provider"]["enabled"], true);
    // The deleted provider's identity can go; the last one can't.
    unlink(id(1)).await.assert(StatusCode::NO_CONTENT);
    unlink(id(1))
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    unlink(id(0))
        .await
        .assert_error(StatusCode::CONFLICT, "last_login_method");
    // Someone else's identity: not found.
    env.app
        .call(
            "DELETE",
            &format!("/api/v1/me/identities/{}", id(0)),
            &env.admin,
            None,
        )
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    // With a password, every identity can go.
    env.app
        .post(
            "/api/v1/me/password",
            &session,
            json!({ "new_password": PASSWORD }),
        )
        .await
        .assert(StatusCode::NO_CONTENT);
    unlink(id(0)).await.assert(StatusCode::NO_CONTENT);
    let resp = env.app.get("/api/v1/me/identities", &session).await;
    assert_eq!(resp.body, json!([]));
    // Devices can't manage identities.
    env.app
        .call(
            "DELETE",
            &format!("/api/v1/me/identities/{}", id(0)),
            &As::Bearer("cld_x".to_owned()),
            None,
        )
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}
