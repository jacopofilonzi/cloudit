//! A fake identity provider for tests: an axum server on a random 127.0.0.1 port.
//! Never contact real providers from tests.
//!
//! - OIDC (issuer `http://127.0.0.1:<port>/oidc`): discovery, JWKS (the keys in
//!   [`FakeIdp::serve_keys`]), token endpoint, user info.
//! - GitHub (`/github/...`) and Discord (`/discord/...`) token and user APIs, wired into
//!   the server with [`FakeIdp::social_endpoints`].
//!
//! There is no authorization page: a test reads `state`, `nonce` and the PKCE
//! challenge from the `start` answer, registers a code with [`FakeIdp::grant`] and calls
//! the callback itself. The token endpoint checks the client credentials (Basic or
//! body), the redirect URI and the PKCE verifier, and each code works once.
//! ID tokens are signed with test RSA keys (`fixtures/test-idp-key-*.pem`, test only).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use openidconnect::core::{
    CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreRsaPrivateSigningKey,
};
use openidconnect::{
    AdditionalClaims, Audience, EndUserEmail, EndUserName, IdToken, IdTokenClaims, IssuerUrl,
    JsonWebKeyId, LocalizedClaim, Nonce, PrivateSigningKey as _, StandardClaims, SubjectIdentifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::idp::social::{OAuth2Endpoints, SocialEndpoints};

const KEY_1: &str = include_str!("fixtures/test-idp-key-1.pem");
const KEY_2: &str = include_str!("fixtures/test-idp-key-2.pem");

pub const CLIENT_ID: &str = "cloudit-test";
pub const CLIENT_SECRET: &str = "fake-client-secret";

/// Extra ID token claims (groups…).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extra(pub serde_json::Map<String, Value>);
impl AdditionalClaims for Extra {}

type TestIdToken =
    IdToken<Extra, CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm>;

/// Which key signs an ID token, and which key id its header names.
#[derive(Debug, Clone, Copy)]
pub enum Signing {
    /// Key 1 as `k1` (served by default).
    Key1,
    /// Key 2 as `k2` (served only after `serve_keys(true, true)`).
    Key2,
    /// Key 2 but named `k1`: a forged signature.
    Forged,
}

/// What the ID token says (defaults: a valid token for `sub`).
#[derive(Debug, Clone)]
pub struct IdClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub extra: serde_json::Map<String, Value>,
    pub nonce: Option<String>,
    pub audience: String,
    /// `None`: the fake's issuer.
    pub issuer: Option<String>,
    /// Seconds from now (negative = expired).
    pub expires_in: i64,
    pub signing: Signing,
}

impl IdClaims {
    pub fn new(sub: &str) -> Self {
        Self {
            sub: sub.to_owned(),
            email: None,
            email_verified: None,
            name: None,
            extra: serde_json::Map::new(),
            nonce: None,
            audience: CLIENT_ID.to_owned(),
            issuer: None,
            expires_in: 300,
            signing: Signing::Key1,
        }
    }

    pub fn email(mut self, email: &str, verified: bool) -> Self {
        self.email = Some(email.to_owned());
        self.email_verified = Some(verified);
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    pub fn groups(mut self, groups: &[&str]) -> Self {
        self.extra.insert("groups".to_owned(), json!(groups));
        self
    }
}

/// One registered authorization code.
struct Grant {
    challenge: String,
    redirect_uri: String,
    access_token: String,
    id_token: Option<String>,
}

#[derive(Default)]
struct FakeState {
    base: String,
    broken: bool,
    serve_key_1: bool,
    serve_key_2: bool,
    grants: HashMap<String, Grant>,
    /// access token → user info JSON.
    userinfo: HashMap<String, Value>,
    /// access token → (`/user`, `/user/emails`).
    github: HashMap<String, (Value, Value)>,
    /// access token → `/users/@me`.
    discord: HashMap<String, Value>,
    /// Number of JWKS fetches.
    jwks_fetches: usize,
}

type Shared = Arc<Mutex<FakeState>>;

/// A running fake provider (stops when the test's runtime ends).
pub struct FakeIdp {
    pub base: String,
    state: Shared,
}

fn key(n: u8) -> CoreRsaPrivateSigningKey {
    let (pem, kid) = if n == 1 { (KEY_1, "k1") } else { (KEY_2, "k2") };
    CoreRsaPrivateSigningKey::from_pem(pem, Some(JsonWebKeyId::new(kid.to_owned()))).unwrap()
}

impl FakeIdp {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state: Shared = Arc::new(Mutex::new(FakeState {
            base: base.clone(),
            serve_key_1: true,
            ..FakeState::default()
        }));
        let app = Router::new()
            .route("/oidc/.well-known/openid-configuration", get(discovery))
            .route("/oidc/jwks", get(jwks))
            .route("/oidc/token", post(token))
            .route("/oidc/userinfo", get(userinfo))
            .route("/github/token", post(token))
            .route("/github/api/user", get(github_user))
            .route("/github/api/user/emails", get(github_emails))
            .route("/discord/token", post(token))
            .route("/discord/api/users/@me", get(discord_user))
            .with_state(Arc::clone(&state));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, state }
    }

    /// The OIDC issuer.
    pub fn issuer(&self) -> String {
        format!("{}/oidc", self.base)
    }

    /// GitHub and Discord pointing at this server.
    pub fn social_endpoints(&self) -> SocialEndpoints {
        let url = |path: &str| reqwest::Url::parse(&format!("{}{path}", self.base)).unwrap();
        SocialEndpoints {
            github: OAuth2Endpoints {
                authorize: url("/github/authorize"),
                token: url("/github/token"),
                api: url("/github/api/"),
            },
            discord: OAuth2Endpoints {
                authorize: url("/discord/authorize"),
                token: url("/discord/token"),
                api: url("/discord/api/"),
            },
        }
    }

    /// Makes discovery fail (500) or work again.
    pub fn set_broken(&self, broken: bool) {
        self.state.lock().unwrap().broken = broken;
    }

    /// Which keys the JWKS lists.
    pub fn serve_keys(&self, key_1: bool, key_2: bool) {
        let mut state = self.state.lock().unwrap();
        state.serve_key_1 = key_1;
        state.serve_key_2 = key_2;
    }

    pub fn jwks_fetches(&self) -> usize {
        self.state.lock().unwrap().jwks_fetches
    }

    /// A signed ID token.
    pub fn id_token(&self, claims: &IdClaims) -> String {
        let issuer = claims.issuer.clone().unwrap_or_else(|| self.issuer());
        let now = chrono::Utc::now();
        let mut standard =
            StandardClaims::<CoreGenderClaim>::new(SubjectIdentifier::new(claims.sub.clone()))
                .set_email(claims.email.clone().map(EndUserEmail::new))
                .set_email_verified(claims.email_verified);
        if let Some(name) = &claims.name {
            let mut localized = LocalizedClaim::new();
            localized.insert(None, EndUserName::new(name.clone()));
            standard = standard.set_name(Some(localized));
        }
        let id_claims = IdTokenClaims::new(
            IssuerUrl::new(issuer).unwrap(),
            vec![Audience::new(claims.audience.clone())],
            now + chrono::TimeDelta::seconds(claims.expires_in),
            now - chrono::TimeDelta::seconds(5),
            standard,
            Extra(claims.extra.clone()),
        )
        .set_nonce(claims.nonce.clone().map(Nonce::new));
        let signer = match claims.signing {
            Signing::Key1 => key(1),
            Signing::Key2 => key(2),
            Signing::Forged => {
                CoreRsaPrivateSigningKey::from_pem(KEY_2, Some(JsonWebKeyId::new("k1".to_owned())))
                    .unwrap()
            }
        };
        TestIdToken::new(
            id_claims,
            &signer,
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            None,
            None,
        )
        .unwrap()
        .to_string()
    }

    /// Registers `code` (single use): the token endpoint answers `access_token` and the
    /// ID token (if any) when called with the matching redirect URI and PKCE verifier.
    pub fn grant(
        &self,
        code: &str,
        challenge: &str,
        redirect_uri: &str,
        access_token: &str,
        id_token: Option<String>,
    ) {
        self.state.lock().unwrap().grants.insert(
            code.to_owned(),
            Grant {
                challenge: challenge.to_owned(),
                redirect_uri: redirect_uri.to_owned(),
                access_token: access_token.to_owned(),
                id_token,
            },
        );
    }

    pub fn set_userinfo(&self, access_token: &str, info: Value) {
        self.state
            .lock()
            .unwrap()
            .userinfo
            .insert(access_token.to_owned(), info);
    }

    /// User info for `access_token` unless already set.
    pub fn default_userinfo(&self, access_token: &str, info: Value) {
        self.state
            .lock()
            .unwrap()
            .userinfo
            .entry(access_token.to_owned())
            .or_insert(info);
    }

    pub fn set_github_user(&self, access_token: &str, user: Value, emails: Value) {
        self.state
            .lock()
            .unwrap()
            .github
            .insert(access_token.to_owned(), (user, emails));
    }

    pub fn set_discord_user(&self, access_token: &str, user: Value) {
        self.state
            .lock()
            .unwrap()
            .discord
            .insert(access_token.to_owned(), user);
    }
}

async fn discovery(State(state): State<Shared>) -> Response {
    let state = state.lock().unwrap();
    if state.broken {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let issuer = format!("{}/oidc", state.base);
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "jwks_uri": format!("{issuer}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
    }))
    .into_response()
}

async fn jwks(State(state): State<Shared>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    state.jwks_fetches += 1;
    let mut keys: Vec<CoreJsonWebKey> = Vec::new();
    if state.serve_key_1 {
        keys.push(key(1).as_verification_key());
    }
    if state.serve_key_2 {
        keys.push(key(2).as_verification_key());
    }
    Json(json!({ "keys": keys }))
}

fn token_error(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// Client credentials from Basic auth (form-decoded) or the body.
fn client_credentials(
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Option<(String, String)> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(encoded) = value.strip_prefix("Basic ")
    {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        let text = String::from_utf8(decoded).ok()?;
        let (id, secret) = text.split_once(':')?;
        let dec = |s: &str| {
            url::form_urlencoded::parse(format!("x={s}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
        };
        return Some((dec(id)?, dec(secret)?));
    }
    Some((
        form.get("client_id")?.clone(),
        form.get("client_secret")?.clone(),
    ))
}

async fn token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if client_credentials(&headers, &form) != Some((CLIENT_ID.to_owned(), CLIENT_SECRET.to_owned()))
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return token_error("unsupported_grant_type");
    }
    let mut state = state.lock().unwrap();
    let Some(grant) = form.get("code").and_then(|c| state.grants.remove(c)) else {
        return token_error("invalid_grant");
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if crate::auth::tokens::pkce_challenge(&verifier) != grant.challenge
        || form.get("redirect_uri") != Some(&grant.redirect_uri)
    {
        return token_error("invalid_grant");
    }
    let mut body = json!({ "access_token": grant.access_token, "token_type": "Bearer" });
    if let Some(id_token) = grant.id_token {
        body["id_token"] = json!(id_token);
    }
    Json(body).into_response()
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

fn found(value: Option<Value>) -> Response {
    match value {
        Some(value) => Json(value).into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn userinfo(State(state): State<Shared>, headers: HeaderMap) -> Response {
    found(
        state
            .lock()
            .unwrap()
            .userinfo
            .get(&bearer(&headers))
            .cloned(),
    )
}

async fn github_user(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if headers.get(header::USER_AGENT).is_none() {
        return StatusCode::FORBIDDEN.into_response(); // GitHub requires one.
    }
    found(
        state
            .lock()
            .unwrap()
            .github
            .get(&bearer(&headers))
            .map(|(u, _)| u.clone()),
    )
}

async fn github_emails(State(state): State<Shared>, headers: HeaderMap) -> Response {
    found(
        state
            .lock()
            .unwrap()
            .github
            .get(&bearer(&headers))
            .map(|(_, e)| e.clone()),
    )
}

async fn discord_user(State(state): State<Shared>, headers: HeaderMap) -> Response {
    found(
        state
            .lock()
            .unwrap()
            .discord
            .get(&bearer(&headers))
            .cloned(),
    )
}
