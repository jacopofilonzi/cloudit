//! Test harness for the API: a full [`AppState`] on a private database (SQLite in a
//! temp dir, or a fresh PostgreSQL database with `TEST_DATABASE_URL`), requests through
//! the real router with `tower::ServiceExt::oneshot`.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use cloudit_core::protocol::{NewAccounts, RegistrationMode};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::accounts::settings;
use crate::accounts::setup::{self, SetupStatus};
use crate::config::Config;
use crate::db::testing::TestDb;
use crate::routes::router;
use crate::state::AppState;

/// The origin the test requests come from (`Host: localhost`), unless `PUBLIC_URL` is
/// set (then its origin).
pub const ORIGIN: &str = "http://localhost";
pub const ADMIN_EMAIL: &str = "admin@example.com";
pub const PASSWORD: &str = "correct horse battery";
/// `SECRET_KEY` used with PostgreSQL (base64 of 32 × 0x01).
pub const TEST_SECRET_KEY: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

/// Credentials of a request.
#[derive(Debug, Clone)]
pub enum As {
    Anon,
    /// Session cookie token.
    Cookie(String),
    /// Device token.
    Bearer(String),
}

/// A response with its JSON body (`Null` if empty).
#[derive(Debug)]
pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Resp {
    /// The error code of an error response.
    pub fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or("")
    }

    /// The session cookie value set by this response (`Some("")` when cleared).
    pub fn cookie(&self) -> Option<String> {
        self.cookie_named("cloudit_session")
    }

    /// The value of cookie `name` set by this response (`Some("")` when cleared).
    pub fn cookie_named(&self, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.strip_prefix(prefix.as_str()))
            .map(|rest| rest.split(';').next().unwrap_or("").to_owned())
    }

    /// The `Location` header.
    pub fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    #[track_caller]
    pub fn assert(&self, status: StatusCode) -> &Self {
        assert_eq!(self.status, status, "body: {}", self.body);
        self
    }

    #[track_caller]
    pub fn assert_error(&self, status: StatusCode, code: &str) {
        assert_eq!(self.status, status, "body: {}", self.body);
        assert_eq!(self.code(), code, "body: {}", self.body);
    }
}

/// A response with its raw body.
#[derive(Debug)]
pub struct RawResp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: bytes::Bytes,
}

impl RawResp {
    #[track_caller]
    pub fn assert(&self, status: StatusCode) -> &Self {
        assert_eq!(
            self.status,
            status,
            "body: {}",
            String::from_utf8_lossy(&self.body)
        );
        self
    }
}

pub struct TestApp {
    pub state: AppState,
    pub app: Router,
    /// Keeps the test database (and its directory) alive.
    _db: TestDb,
    /// `Origin` sent by [`TestApp::request`].
    pub origin: String,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with(&[]).await
    }

    /// With extra environment variables.
    pub async fn with(extra: &[(&str, &str)]) -> Self {
        let db = TestDb::empty().await;
        let mut vars: Vec<(String, String)> = vec![
            (
                "WEB_DIR".to_owned(),
                db.dir.path().join("no-web").to_string_lossy().into_owned(),
            ),
            (
                "DATA_DIR".to_owned(),
                db.dir.path().to_string_lossy().into_owned(),
            ),
        ];
        if let Some(url) = db.url() {
            vars.push(("DATABASE_URL".to_owned(), url.to_owned()));
        }
        vars.extend(
            extra
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        // PostgreSQL refuses to start without a key (SQLite generates its key file).
        if db.url().is_some() && !extra.iter().any(|(k, _)| *k == "SECRET_KEY") {
            vars.push(("SECRET_KEY".to_owned(), TEST_SECRET_KEY.to_owned()));
        }
        let mut state = AppState::init(Self::config(&vars)).await.unwrap();
        // 2 min 30 s past a whole hour: mid-window for every limit (5 min, 15 min, 1 h).
        state.rate_limiter = state
            .rate_limiter
            .clone()
            .with_fixed_clock(1_700_002_950_000);
        let app = router(state.clone());
        let origin = state.config.public_url.as_deref().map_or_else(
            || ORIGIN.to_owned(),
            |u| url::Url::parse(u).unwrap().origin().ascii_serialization(),
        );
        Self {
            state,
            app,
            _db: db,
            origin,
        }
    }

    /// With GitHub and Discord pointing at `idp`.
    pub async fn with_fake_idp(extra: &[(&str, &str)], idp: &super::fake_idp::FakeIdp) -> Self {
        let mut test = Self::with(extra).await;
        test.state.idp.set_social_endpoints(idp.social_endpoints());
        test.app = router(test.state.clone());
        test
    }

    pub fn config(vars: &[(String, String)]) -> Config {
        Config::from_lookup(|key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
            .unwrap()
    }

    /// The configuration the state was built with (for the CLI functions).
    pub fn cli_config(&self) -> Config {
        (*self.state.config).clone()
    }

    pub async fn send(&self, request: Request<Body>) -> Resp {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Resp {
            status,
            headers,
            body,
        }
    }

    /// A browser-like request from the server's own origin.
    pub fn request(
        &self,
        method: &str,
        uri: &str,
        who: &As,
        body: Option<&Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, &self.origin)
            .header(header::USER_AGENT, "cloudit-test");
        builder = match who {
            As::Anon => builder,
            As::Cookie(token) => builder.header(header::COOKIE, format!("cloudit_session={token}")),
            As::Bearer(token) => builder.header(header::AUTHORIZATION, format!("Bearer {token}")),
        };
        match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    /// A request with a raw body of `content_type`.
    pub fn raw_request(
        &self,
        method: &str,
        uri: &str,
        who: &As,
        content_type: &str,
        body: Vec<u8>,
    ) -> Request<Body> {
        let mut request = self.request(method, uri, who, None);
        request
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
        *request.body_mut() = Body::from(body);
        request
    }

    /// Sends a raw body (`application/octet-stream`); the answer as JSON.
    pub fn send_raw<'a>(
        &'a self,
        method: &str,
        uri: &str,
        who: &As,
        body: Vec<u8>,
    ) -> impl Future<Output = Resp> + use<'a> {
        self.send(self.raw_request(method, uri, who, "application/octet-stream", body))
    }

    /// Sends `request`; the answer with its raw body.
    pub async fn send_for_bytes(&self, request: Request<Body>) -> RawResp {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        RawResp {
            status,
            headers,
            body,
        }
    }

    /// `GET uri`; the answer with its raw body.
    pub fn get_bytes<'a>(&'a self, uri: &str, who: &As) -> impl Future<Output = RawResp> + use<'a> {
        self.send_for_bytes(self.request("GET", uri, who, None))
    }

    /// Builds the request now, so the future borrows only `self`.
    pub fn call<'a>(
        &'a self,
        method: &str,
        uri: &str,
        who: &As,
        body: Option<Value>,
    ) -> impl Future<Output = Resp> + use<'a> {
        self.send(self.request(method, uri, who, body.as_ref()))
    }

    pub fn get<'a>(&'a self, uri: &str, who: &As) -> impl Future<Output = Resp> + use<'a> {
        self.call("GET", uri, who, None)
    }

    pub fn post<'a>(
        &'a self,
        uri: &str,
        who: &As,
        body: Value,
    ) -> impl Future<Output = Resp> + use<'a> {
        self.call("POST", uri, who, Some(body))
    }

    /// The pending setup token, as `cloudit-server setup-token` reads it.
    pub async fn setup_token(&self) -> String {
        match setup::status(&self.state.db).await.unwrap() {
            SetupStatus::Pending(token) => token.expose().to_owned(),
            other => panic!("setup not pending: {other:?}"),
        }
    }

    /// Completes setup; returns the admin's session cookie.
    pub async fn setup_admin(&self) -> As {
        let token = self.setup_token().await;
        let resp = self
            .post(
                "/api/v1/setup",
                &As::Anon,
                json!({
                    "token": token, "email": ADMIN_EMAIL, "password": PASSWORD,
                    "display_name": "Admin"
                }),
            )
            .await;
        resp.assert(StatusCode::CREATED);
        As::Cookie(resp.cookie().unwrap())
    }

    pub async fn set_password_policy(&self, policy: NewAccounts) {
        settings::set_password_new_accounts(&self.state.db, policy, None)
            .await
            .unwrap();
    }

    pub async fn set_mode(&self, mode: RegistrationMode) {
        settings::set_registration_mode(&self.state.db, mode, None)
            .await
            .unwrap();
    }

    /// Registers `email` (registration temporarily open, without approval); returns
    /// its session cookie.
    pub async fn user(&self, email: &str) -> As {
        let previous = settings::registration_mode(&self.state.db).await.unwrap();
        let policy = settings::password_new_accounts(&self.state.db)
            .await
            .unwrap();
        self.set_mode(RegistrationMode::Open).await;
        self.set_password_policy(NewAccounts::Auto).await;
        let resp = self
            .post(
                "/api/v1/auth/register",
                &As::Anon,
                json!({ "email": email, "password": PASSWORD, "display_name": email }),
            )
            .await;
        resp.assert(StatusCode::CREATED);
        self.set_mode(previous).await;
        self.set_password_policy(policy).await;
        As::Cookie(resp.cookie().unwrap())
    }

    pub async fn login(&self, email: &str, password: &str) -> Resp {
        self.post(
            "/api/v1/auth/login",
            &As::Anon,
            json!({ "email": email, "password": password }),
        )
        .await
    }

    /// The id of the user signed in as `who`.
    pub async fn user_id(&self, who: &As) -> String {
        let resp = self.get("/api/v1/me", who).await;
        resp.assert(StatusCode::OK);
        resp.body["id"].as_str().unwrap().to_owned()
    }
}
