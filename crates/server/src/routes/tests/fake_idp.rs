//! A fake identity provider for tests: an axum server on a random 127.0.0.1 port with
//! OIDC discovery (issuer `http://127.0.0.1:<port>/oidc`) and JWKS. Never contact real
//! providers from tests.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

#[derive(Default)]
struct FakeState {
    base: String,
    /// Answer 500 to discovery requests.
    broken: bool,
}

type Shared = Arc<Mutex<FakeState>>;

/// A running fake provider (stops when the test's runtime ends).
pub struct FakeIdp {
    pub base: String,
    state: Shared,
}

impl FakeIdp {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state: Shared = Arc::new(Mutex::new(FakeState {
            base: base.clone(),
            ..FakeState::default()
        }));
        let app = Router::new()
            .route("/oidc/.well-known/openid-configuration", get(discovery))
            .route("/oidc/jwks", get(jwks))
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

    /// Makes discovery fail (500) or work again.
    pub fn set_broken(&self, broken: bool) {
        self.state.lock().unwrap().broken = broken;
    }
}

async fn discovery(State(state): State<Shared>) -> Response {
    let state = state.lock().unwrap();
    if state.broken {
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
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
    }))
    .into_response()
}

async fn jwks() -> Json<Value> {
    Json(json!({ "keys": [] }))
}
