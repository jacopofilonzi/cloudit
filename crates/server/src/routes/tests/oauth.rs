//! Provider sign-in: start → callback for OIDC, GitHub and Discord against the fake
//! provider; flow state rules, ID token checks, groups, auto-link, linking.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::{Value, json};

use super::fake_idp::{CLIENT_ID, CLIENT_SECRET, FakeIdp, IdClaims, Signing};
use super::support::{ADMIN_EMAIL, As, PASSWORD, Resp, TestApp};
use crate::db::entities::{identities, oauth_flows, pending_signups, users};

pub const PUBLIC: &str = "https://sync.example.com";

/// The values a `start` call handed to the provider.
pub struct Started {
    pub state: String,
    pub nonce: Option<String>,
    pub challenge: String,
    pub binding: String,
    pub url: url::Url,
}

/// A test app with the fake provider, an admin, a public URL and three providers:
/// `sso` (generic OIDC, admin group `admins`), `gh` (GitHub), `dc` (Discord).
pub struct Env {
    pub app: TestApp,
    pub idp: FakeIdp,
    pub admin: As,
}

impl Env {
    pub async fn new() -> Self {
        let idp = FakeIdp::start().await;
        let app = TestApp::with_fake_idp(&[], &idp).await;
        let admin = app.setup_admin().await;
        crate::accounts::settings::set_public_url(&app.state.db, Some(PUBLIC), None)
            .await
            .unwrap();
        let env = Self { app, idp, admin };
        env.add_provider(json!({
            "id": "sso", "kind": "oidc", "issuer_url": env.idp.issuer(),
            "admin_groups": ["admins"]
        }))
        .await;
        env.add_provider(json!({ "id": "gh", "kind": "github" }))
            .await;
        env.add_provider(json!({ "id": "dc", "kind": "discord" }))
            .await;
        env
    }

    /// Creates a provider with the fake's client credentials (`new_accounts`: `auto`
    /// unless given).
    pub async fn add_provider(&self, mut body: Value) {
        if body.get("new_accounts").is_none() {
            body["new_accounts"] = json!("auto");
        }
        body["client_id"] = json!(CLIENT_ID);
        body["client_secret"] = json!(CLIENT_SECRET);
        self.app
            .post("/api/v1/admin/providers", &self.admin, body)
            .await
            .assert(StatusCode::CREATED);
    }

    pub fn redirect_uri(provider: &str) -> String {
        format!("{PUBLIC}/api/v1/auth/oauth/{provider}/callback")
    }

    pub async fn start_with(&self, provider: &str, body: Value, who: &As) -> Resp {
        self.app
            .post(&format!("/api/v1/auth/oauth/{provider}/start"), who, body)
            .await
    }

    pub fn started(resp: &Resp) -> Started {
        resp.assert(StatusCode::OK);
        let url = url::Url::parse(resp.body["redirect_url"].as_str().unwrap()).unwrap();
        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        Started {
            state: param("state").unwrap(),
            nonce: param("nonce"),
            challenge: param("code_challenge").unwrap(),
            binding: resp.cookie_named("cloudit_oauth").unwrap(),
            url,
        }
    }

    pub async fn start(&self, provider: &str, purpose: &str, who: &As) -> Started {
        Self::started(
            &self
                .start_with(
                    provider,
                    json!({ "purpose": purpose, "return_to": "/after" }),
                    who,
                )
                .await,
        )
    }

    /// `GET …/callback?<query>` with the binding cookie (and the session of `who`).
    pub async fn callback(
        &self,
        provider: &str,
        query: &str,
        binding: Option<&str>,
        who: &As,
    ) -> Resp {
        let mut cookies = Vec::new();
        if let Some(binding) = binding {
            cookies.push(format!("cloudit_oauth={binding}"));
        }
        if let As::Cookie(session) = who {
            cookies.push(format!("cloudit_session={session}"));
        }
        let mut builder = Request::get(format!("/api/v1/auth/oauth/{provider}/callback?{query}"))
            .header(header::HOST, "localhost");
        if !cookies.is_empty() {
            builder = builder.header(header::COOKIE, cookies.join("; "));
        }
        self.app.send(builder.body(Body::empty()).unwrap()).await
    }

    /// Registers a code for `started` and calls the callback with it.
    pub async fn finish(
        &self,
        provider: &str,
        started: &Started,
        id_token: Option<String>,
        access: &str,
        who: &As,
    ) -> Resp {
        let code = uuid::Uuid::new_v4().simple().to_string();
        self.idp.grant(
            &code,
            &started.challenge,
            &Self::redirect_uri(provider),
            access,
            id_token,
        );
        self.callback(
            provider,
            &format!("code={code}&state={}", started.state),
            Some(&started.binding),
            who,
        )
        .await
    }

    /// A whole OIDC round trip through `sso` (the nonce is filled in unless set). The
    /// access token is `oidc-<sub>`; its user info defaults to `{"sub": <sub>}`.
    pub async fn oidc(&self, mut claims: IdClaims, purpose: &str, who: &As) -> Resp {
        let started = self.start("sso", purpose, who).await;
        if claims.nonce.is_none() {
            claims.nonce.clone_from(&started.nonce);
        }
        let token = self.idp.id_token(&claims);
        let access = format!("oidc-{}", claims.sub);
        self.idp
            .default_userinfo(&access, json!({ "sub": claims.sub }));
        self.finish("sso", &started, Some(token), &access, who)
            .await
    }

    pub async fn github(&self, id: u64, email: Option<&str>, purpose: &str, who: &As) -> Resp {
        let access = format!("gh-{id}");
        let emails = match email {
            Some(e) => json!([
                { "email": "old@example.com", "primary": false, "verified": true },
                { "email": e, "primary": true, "verified": true }
            ]),
            None => {
                json!([{ "email": "unverified@example.com", "primary": true, "verified": false }])
            }
        };
        self.idp.set_github_user(
            &access,
            json!({ "id": id, "login": format!("octo{id}"), "name": null }),
            emails,
        );
        let started = self.start("gh", purpose, who).await;
        self.finish("gh", &started, None, &access, who).await
    }

    pub async fn discord(
        &self,
        id: &str,
        email: &str,
        verified: bool,
        purpose: &str,
        who: &As,
    ) -> Resp {
        let access = format!("dc-{id}");
        self.idp.set_discord_user(
            &access,
            json!({ "id": id, "username": "wumpus", "global_name": "Wumpus", "email": email, "verified": verified }),
        );
        let started = self.start("dc", purpose, who).await;
        self.finish("dc", &started, None, &access, who).await
    }

    pub async fn identity_rows(&self) -> Vec<identities::Model> {
        identities::Entity::find()
            .all(&self.app.state.db)
            .await
            .unwrap()
    }

    pub async fn user_by_email(&self, email: &str) -> users::Model {
        crate::accounts::users::find_by_email(&self.app.state.db, email)
            .await
            .unwrap()
            .unwrap()
    }
}

/// The error code of a `/login?error=` redirect.
#[track_caller]
pub fn assert_login_error(resp: &Resp, code: &str) {
    assert_eq!(resp.status, StatusCode::FOUND, "body: {}", resp.body);
    assert_eq!(resp.location(), format!("/login?error={code}"));
    assert!(resp.cookie().is_none(), "no session on error");
}

/// A signed-in redirect: returns the new session cookie.
#[track_caller]
pub fn assert_signed_in(resp: &Resp, location: &str) -> As {
    assert_eq!(resp.status, StatusCode::FOUND, "body: {}", resp.body);
    assert_eq!(resp.location(), location);
    As::Cookie(resp.cookie().expect("session cookie"))
}

#[tokio::test]
async fn start_validates_and_sets_the_binding_cookie() {
    let env = Env::new().await;
    let resp = env
        .start_with(
            "sso",
            json!({ "purpose": "login", "return_to": "/x" }),
            &As::Anon,
        )
        .await;
    let started = Env::started(&resp);
    let cookie = resp
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| v.to_str().ok().filter(|v| v.starts_with("cloudit_oauth=")))
        .unwrap()
        .to_owned();
    assert!(
        cookie.ends_with("; Path=/api/v1/auth; HttpOnly; SameSite=Lax; Max-Age=600"),
        "{cookie}"
    );
    let url = &started.url;
    assert_eq!(
        url.as_str().split('?').next(),
        Some(format!("{}/authorize", env.idp.issuer()).as_str())
    );
    let params: std::collections::HashMap<String, String> =
        url.query_pairs().into_owned().collect();
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["redirect_uri"], Env::redirect_uri("sso"));
    assert_eq!(params["scope"], "openid email profile");
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["code_challenge"].len(), 43);
    assert!(started.nonce.is_some());
    // The flow stores hashes of state and binding, not the values.
    let flow = oauth_flows::Entity::find()
        .one(&env.app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        flow.state_hash,
        crate::auth::tokens::hash_token(&started.state)
    );
    assert_ne!(flow.browser_binding_hash, started.binding);
    assert_eq!(flow.return_to, "/x");

    // GitHub: no nonce, its own authorize URL and scopes.
    let gh = env.start("gh", "login", &As::Anon).await;
    assert!(gh.nonce.is_none());
    assert!(
        gh.url
            .as_str()
            .starts_with(&format!("{}/github/authorize?", env.idp.base))
    );
    assert!(
        gh.url
            .query_pairs()
            .any(|(k, v)| k == "scope" && v == "read:user user:email")
    );

    for bad in ["https://evil.example", "//evil.example", "relative"] {
        let resp = env
            .start_with(
                "sso",
                json!({ "purpose": "login", "return_to": bad }),
                &As::Anon,
            )
            .await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(resp.body["field"], "return_to");
    }
    env.start_with("nope", json!({ "purpose": "login" }), &As::Anon)
        .await
        .assert_error(StatusCode::NOT_FOUND, "provider_not_found");
    env.start_with("sso", json!({ "purpose": "link" }), &As::Anon)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    env.app
        .call(
            "PATCH",
            "/api/v1/admin/providers/gh",
            &env.admin,
            Some(json!({ "enabled": false })),
        )
        .await
        .assert(StatusCode::OK);
    env.start_with("gh", json!({ "purpose": "login" }), &As::Anon)
        .await
        .assert_error(StatusCode::FORBIDDEN, "provider_disabled");
    // Cross-site POST: refused before anything else.
    let request = Request::post("/api/v1/auth/oauth/sso/start")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "purpose": "login" }).to_string()))
        .unwrap();
    env.app
        .send(request)
        .await
        .assert_error(StatusCode::FORBIDDEN, "csrf_failed");
    // Without a public URL there is no callback URL.
    crate::accounts::settings::set_public_url(&env.app.state.db, None, None)
        .await
        .unwrap();
    env.start_with("sso", json!({ "purpose": "login" }), &As::Anon)
        .await
        .assert_error(StatusCode::CONFLICT, "public_url_required");
}

#[tokio::test]
async fn oidc_link_then_sign_in() {
    let env = Env::new().await;
    let admin_id = env.app.user_id(&env.admin).await;
    // Link from the admin's session.
    let resp = env
        .oidc(
            IdClaims::new("sub-1").email("Admin.SSO@Example.com", true),
            "link",
            &env.admin,
        )
        .await;
    assert_eq!(resp.status, StatusCode::FOUND, "{}", resp.body);
    assert_eq!(resp.location(), "/after");
    assert!(resp.cookie().is_none(), "linking doesn't create a session");
    let rows = env.identity_rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id.to_string(), admin_id);
    assert_eq!(rows[0].provider_id, "sso");
    assert_eq!(rows[0].subject, "sub-1");
    assert_eq!(rows[0].email.as_deref(), Some("admin.sso@example.com"));
    assert!(rows[0].last_login_at.is_none());
    // Linking again from the same account: a no-op.
    let resp = env.oidc(IdClaims::new("sub-1"), "link", &env.admin).await;
    assert_eq!(resp.location(), "/after");
    assert_eq!(env.identity_rows().await.len(), 1);

    // Sign in with it: session for the admin, identity updated.
    let resp = env
        .oidc(
            IdClaims::new("sub-1").email("new@example.com", false),
            "login",
            &As::Anon,
        )
        .await;
    let session = assert_signed_in(&resp, "/after");
    assert_eq!(env.app.user_id(&session).await, admin_id);
    let rows = env.identity_rows().await;
    assert_eq!(rows[0].email.as_deref(), Some("new@example.com"));
    assert!(!rows[0].email_verified);
    assert!(rows[0].last_login_at.is_some());
    // The account's own email is not changed by the provider.
    assert_eq!(env.user_by_email(ADMIN_EMAIL).await.email, ADMIN_EMAIL);
}

#[tokio::test]
async fn github_and_discord_sign_in() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    let user_id = env.app.user_id(&user).await;
    let resp = env
        .github(42, Some("octo@example.com"), "link", &user)
        .await;
    assert_eq!(resp.location(), "/after", "{}", resp.body);
    let resp = env
        .discord("1234", "wumpus@example.com", true, "link", &user)
        .await;
    assert_eq!(resp.location(), "/after", "{}", resp.body);
    let mut rows = env.identity_rows().await;
    rows.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
    assert_eq!(rows[0].provider_id, "dc");
    assert_eq!(rows[0].subject, "1234");
    assert_eq!(rows[0].email.as_deref(), Some("wumpus@example.com"));
    assert!(rows[0].email_verified);
    assert_eq!(rows[1].provider_id, "gh");
    assert_eq!(rows[1].subject, "42");
    // The primary verified address, not another verified one.
    assert_eq!(rows[1].email.as_deref(), Some("octo@example.com"));

    let session = assert_signed_in(&env.github(42, None, "login", &As::Anon).await, "/after");
    assert_eq!(env.app.user_id(&session).await, user_id);
    let session = assert_signed_in(
        &env.discord("1234", "wumpus@example.com", false, "login", &As::Anon)
            .await,
        "/after",
    );
    assert_eq!(env.app.user_id(&session).await, user_id);
    // GitHub without a primary verified email: none recorded.
    let gh = env
        .identity_rows()
        .await
        .into_iter()
        .find(|r| r.provider_id == "gh")
        .unwrap();
    assert_eq!(gh.email, None);
    assert!(!gh.email_verified);
}

#[tokio::test]
async fn unknown_identity_goes_to_onboarding() {
    let env = Env::new().await;
    let resp = env
        .oidc(
            IdClaims::new("new-sub")
                .email("New@Example.com", true)
                .name("New Person")
                .groups(&["staff", "admins"]),
            "login",
            &As::Anon,
        )
        .await;
    assert_eq!(resp.status, StatusCode::FOUND, "{}", resp.body);
    assert_eq!(resp.location(), "/onboarding");
    assert!(resp.cookie().is_none());
    let token = resp.cookie_named("cloudit_onboarding").unwrap();
    let pending = pending_signups::Entity::find()
        .one(&env.app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.token_hash, crate::auth::tokens::hash_token(&token));
    assert_eq!(pending.provider_id, "sso");
    assert_eq!(pending.subject, "new-sub");
    assert_eq!(pending.email.as_deref(), Some("new@example.com"));
    assert!(pending.email_verified);
    assert_eq!(pending.display_name, "New Person");
    assert_eq!(pending.groups, r#"["staff","admins"]"#);
    assert!(env.identity_rows().await.is_empty());
}

#[tokio::test]
async fn flow_state_rules() {
    let env = Env::new().await;
    // Reuse: the second callback with the same state fails.
    let started = env.start("gh", "login", &As::Anon).await;
    env.idp
        .set_github_user("gh-a", json!({ "id": 1, "login": "a" }), json!([]));
    let resp = env.finish("gh", &started, None, "gh-a", &As::Anon).await;
    assert_eq!(resp.location(), "/onboarding");
    let resp = env.finish("gh", &started, None, "gh-a", &As::Anon).await;
    assert_login_error(&resp, "oauth_state_invalid");

    // Another browser (no or another binding cookie) can't complete it, and doesn't
    // burn it: the starting browser still can.
    let started = env.start("gh", "login", &As::Anon).await;
    let query = format!("code=x&state={}", started.state);
    assert_login_error(
        &env.callback("gh", &query, None, &As::Anon).await,
        "oauth_state_invalid",
    );
    let other = "b".repeat(43);
    assert_login_error(
        &env.callback("gh", &query, Some(&other), &As::Anon).await,
        "oauth_state_invalid",
    );
    // Wrong provider in the path.
    assert_login_error(
        &env.callback("dc", &query, Some(&started.binding), &As::Anon)
            .await,
        "oauth_state_invalid",
    );
    let resp = env.finish("gh", &started, None, "gh-a", &As::Anon).await;
    assert_eq!(resp.location(), "/onboarding");

    // Unknown state, expired flow.
    assert_login_error(
        &env.callback(
            "gh",
            "code=x&state=made-up",
            Some(&started.binding),
            &As::Anon,
        )
        .await,
        "oauth_state_invalid",
    );
    let started = env.start("gh", "login", &As::Anon).await;
    oauth_flows::Entity::update_many()
        .col_expr(
            oauth_flows::Column::ExpiresAt,
            Expr::value(crate::time::now() - chrono::TimeDelta::seconds(1)),
        )
        .exec(&env.app.state.db)
        .await
        .unwrap();
    assert_login_error(
        &env.finish("gh", &started, None, "gh-a", &As::Anon).await,
        "oauth_state_invalid",
    );

    // The user cancels at the provider; other provider errors.
    let started = env.start("gh", "login", &As::Anon).await;
    let resp = env
        .callback(
            "gh",
            &format!("error=access_denied&state={}", started.state),
            Some(&started.binding),
            &As::Anon,
        )
        .await;
    assert_login_error(&resp, "oauth_cancelled");
    let started = env.start("gh", "login", &As::Anon).await;
    let resp = env
        .callback(
            "gh",
            &format!("error=server_error&state={}", started.state),
            Some(&started.binding),
            &As::Anon,
        )
        .await;
    assert_login_error(&resp, "provider_error");
    // Unknown code at the token endpoint.
    let started = env.start("gh", "login", &As::Anon).await;
    let resp = env
        .callback(
            "gh",
            &format!("code=never-granted&state={}", started.state),
            Some(&started.binding),
            &As::Anon,
        )
        .await;
    assert_login_error(&resp, "provider_error");
}

#[tokio::test]
async fn id_token_checks() {
    let env = Env::new().await;
    // Link a known identity first, so failures can't be confused with onboarding.
    let resp = env.oidc(IdClaims::new("s"), "link", &env.admin).await;
    assert_eq!(resp.location(), "/after");
    let mut cases = Vec::new();
    let mut bad_nonce = IdClaims::new("s");
    bad_nonce.nonce = Some("another-nonce".to_owned());
    cases.push(bad_nonce);
    let mut forged = IdClaims::new("s");
    forged.signing = Signing::Forged;
    cases.push(forged);
    let mut audience = IdClaims::new("s");
    audience.audience = "someone-else".to_owned();
    cases.push(audience);
    let mut issuer = IdClaims::new("s");
    issuer.issuer = Some("https://evil.example".to_owned());
    cases.push(issuer);
    let mut expired = IdClaims::new("s");
    expired.expires_in = -60;
    cases.push(expired);
    for claims in cases {
        let resp = env.oidc(claims.clone(), "login", &As::Anon).await;
        assert_eq!(resp.location(), "/login?error=provider_error", "{claims:?}");
    }
    // Missing nonce claim.
    let started = env.start("sso", "login", &As::Anon).await;
    let token = env.idp.id_token(&IdClaims::new("s"));
    let resp = env
        .finish("sso", &started, Some(token), "a", &As::Anon)
        .await;
    assert_login_error(&resp, "provider_error");
    // No ID token at all.
    let started = env.start("sso", "login", &As::Anon).await;
    let resp = env.finish("sso", &started, None, "a", &As::Anon).await;
    assert_login_error(&resp, "provider_error");
    // The valid one still works.
    assert_signed_in(
        &env.oidc(IdClaims::new("s"), "login", &As::Anon).await,
        "/after",
    );
}

#[tokio::test]
async fn rotated_keys_are_refetched_once() {
    let env = Env::new().await;
    env.oidc(IdClaims::new("s"), "link", &env.admin).await;
    let fetches = env.idp.jwks_fetches();
    // The provider rotates to key 2: the cached JWKS lacks `k2`.
    env.idp.serve_keys(false, true);
    let mut claims = IdClaims::new("s");
    claims.signing = Signing::Key2;
    assert_signed_in(
        &env.oidc(claims.clone(), "login", &As::Anon).await,
        "/after",
    );
    assert_eq!(env.idp.jwks_fetches(), fetches + 1);
    // Cached now.
    assert_signed_in(&env.oidc(claims, "login", &As::Anon).await, "/after");
    assert_eq!(env.idp.jwks_fetches(), fetches + 1);
}

#[tokio::test]
async fn userinfo_fills_missing_email_and_groups() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    env.oidc(IdClaims::new("u"), "link", &user).await;
    env.idp.set_userinfo(
        "oidc-u",
        json!({ "sub": "u", "email": "info@example.com", "email_verified": "true", "groups": "admins" }),
    );
    // No email in the ID token: from user info (string boolean accepted), with groups.
    assert_signed_in(
        &env.oidc(IdClaims::new("u"), "login", &As::Anon).await,
        "/after",
    );
    let row = env
        .identity_rows()
        .await
        .into_iter()
        .find(|r| r.subject == "u")
        .unwrap();
    assert_eq!(row.email.as_deref(), Some("info@example.com"));
    assert!(row.email_verified);
    assert!(
        env.user_by_email("user@example.com")
            .await
            .admin_from_groups
    );
    // …but user info about another subject is refused.
    env.idp.set_userinfo(
        "oidc-u",
        json!({ "sub": "someone-else", "email": "x@example.com" }),
    );
    assert_login_error(
        &env.oidc(IdClaims::new("u"), "login", &As::Anon).await,
        "provider_error",
    );
}

#[tokio::test]
async fn allowed_groups_and_admin_from_groups() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    let user_id = env.app.user_id(&user).await;
    env.oidc(IdClaims::new("u"), "link", &user).await;
    let is_group_admin = || async {
        env.user_by_email("user@example.com")
            .await
            .admin_from_groups
    };

    // In `admins`: becomes admin; without it: not any more.
    let session = assert_signed_in(
        &env.oidc(IdClaims::new("u").groups(&["admins"]), "login", &As::Anon)
            .await,
        "/after",
    );
    assert!(is_group_admin().await);
    env.app
        .get("/api/v1/admin/users", &session)
        .await
        .assert(StatusCode::OK);
    assert_signed_in(
        &env.oidc(IdClaims::new("u").groups(&["staff"]), "login", &As::Anon)
            .await,
        "/after",
    );
    assert!(!is_group_admin().await);
    // GitHub sign-ins leave it as it is.
    assert_signed_in(
        &env.oidc(IdClaims::new("u").groups(&["admins"]), "login", &As::Anon)
            .await,
        "/after",
    );
    env.github(7, None, "link", &user).await;
    assert_signed_in(&env.github(7, None, "login", &As::Anon).await, "/after");
    assert!(is_group_admin().await);

    // Last-admin guard: once the user is the only admin, losing the group keeps it.
    let admin_id = env.app.user_id(&env.admin).await;
    env.app
        .call(
            "PATCH",
            &format!("/api/v1/admin/users/{admin_id}"),
            &session,
            Some(json!({ "admin": false })),
        )
        .await
        .assert(StatusCode::OK);
    assert_signed_in(
        &env.oidc(IdClaims::new("u"), "login", &As::Anon).await,
        "/after",
    );
    assert!(is_group_admin().await, "kept: it would leave no admin");

    // allowed_groups: only members may sign in.
    env.app
        .call(
            "PATCH",
            "/api/v1/admin/providers/sso",
            &session,
            Some(json!({ "allowed_groups": ["staff"] })),
        )
        .await
        .assert(StatusCode::OK);
    assert_login_error(
        &env.oidc(IdClaims::new("u").groups(&["admins"]), "login", &As::Anon)
            .await,
        "group_not_allowed",
    );
    assert_login_error(
        &env.oidc(IdClaims::new("u"), "login", &As::Anon).await,
        "group_not_allowed",
    );
    let s = assert_signed_in(
        &env.oidc(IdClaims::new("u").groups(&["staff"]), "login", &As::Anon)
            .await,
        "/after",
    );
    assert_eq!(env.app.user_id(&s).await, user_id);
}

#[tokio::test]
async fn autolink_trusts_only_verified_emails() {
    let env = Env::new().await;
    // The admin's email is verified (first-run setup); a registered user's isn't.
    env.app.user("user@example.com").await;
    let set_autolink = |on: bool| {
        env.app.call(
            "PATCH",
            "/api/v1/admin/settings",
            &env.admin,
            Some(json!({ "autolink_email": on })),
        )
    };
    // Off (default): onboarding even for a verified match.
    let resp = env
        .oidc(
            IdClaims::new("a").email(ADMIN_EMAIL, true),
            "login",
            &As::Anon,
        )
        .await;
    assert_eq!(resp.location(), "/onboarding");
    set_autolink(true).await.assert(StatusCode::OK);
    // Provider doesn't vouch for the email: onboarding.
    let resp = env
        .oidc(
            IdClaims::new("a").email(ADMIN_EMAIL, false),
            "login",
            &As::Anon,
        )
        .await;
    assert_eq!(resp.location(), "/onboarding");
    // The account's email is unverified: onboarding.
    let resp = env
        .oidc(
            IdClaims::new("b").email("user@example.com", true),
            "login",
            &As::Anon,
        )
        .await;
    assert_eq!(resp.location(), "/onboarding");
    // GitHub's primary verified email matches the admin: linked and signed in.
    let session = assert_signed_in(
        &env.github(9, Some(ADMIN_EMAIL), "login", &As::Anon).await,
        "/after",
    );
    assert_eq!(
        env.app.user_id(&session).await,
        env.app.user_id(&env.admin).await
    );
    // Both verified: linked and signed in.
    let session = assert_signed_in(
        &env.oidc(
            IdClaims::new("a").email(ADMIN_EMAIL, true),
            "login",
            &As::Anon,
        )
        .await,
        "/after",
    );
    assert_eq!(
        env.app.user_id(&session).await,
        env.app.user_id(&env.admin).await
    );
    let linked: Vec<String> = env
        .identity_rows()
        .await
        .into_iter()
        .map(|r| r.subject)
        .collect();
    assert_eq!(linked.len(), 2);
}

#[tokio::test]
async fn linking_rules() {
    let env = Env::new().await;
    env.idp.default_userinfo("a", json!({ "sub": "mine" }));
    let user = env.app.user("user@example.com").await;
    env.oidc(IdClaims::new("shared"), "link", &env.admin).await;
    // Already linked to another account.
    assert_login_error(
        &env.oidc(IdClaims::new("shared"), "link", &user).await,
        "identity_in_use",
    );
    // The session changed between start and callback: refused.
    let started = env.start("sso", "link", &user).await;
    let token = env.idp.id_token(&{
        let mut c = IdClaims::new("mine");
        c.nonce.clone_from(&started.nonce);
        c
    });
    assert_login_error(
        &env.finish("sso", &started, Some(token.clone()), "a", &env.admin)
            .await,
        "oauth_state_invalid",
    );
    let started = env.start("sso", "link", &user).await;
    let token = env.idp.id_token(&{
        let mut c = IdClaims::new("mine");
        c.nonce.clone_from(&started.nonce);
        c
    });
    let resp = env.finish("sso", &started, Some(token), "a", &user).await;
    assert_eq!(resp.location(), "/after");
    let rows = env.identity_rows().await;
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn disabled_accounts_cant_sign_in_with_a_provider() {
    let env = Env::new().await;
    let user = env.app.user("user@example.com").await;
    let id = env.app.user_id(&user).await;
    env.github(5, None, "link", &user).await;
    env.app
        .call(
            "PATCH",
            &format!("/api/v1/admin/users/{id}"),
            &env.admin,
            Some(json!({ "disabled": true })),
        )
        .await
        .assert(StatusCode::OK);
    assert_login_error(
        &env.github(5, None, "login", &As::Anon).await,
        "account_disabled",
    );
    // Re-enabled: works again, and the password still does too.
    env.app
        .call(
            "PATCH",
            &format!("/api/v1/admin/users/{id}"),
            &env.admin,
            Some(json!({ "disabled": false })),
        )
        .await
        .assert(StatusCode::OK);
    assert_signed_in(&env.github(5, None, "login", &As::Anon).await, "/after");
    env.app
        .login("user@example.com", PASSWORD)
        .await
        .assert(StatusCode::OK);
    // Users list unaffected by the session filter.
    let count = users::Entity::find()
        .filter(users::Column::Email.eq("user@example.com"))
        .all(&env.app.state.db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1);
}
