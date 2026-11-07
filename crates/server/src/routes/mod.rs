//! HTTP router.
//!
//! Layout:
//! - `GET /health` (liveness) and `GET /ready` (readiness) at the root, for probes
//!   and the Docker `HEALTHCHECK`;
//! - the versioned API under `/api/v1`; anything else under `/api` answers a JSON
//!   `not_found` error instead of falling through to the SPA;
//! - everything else serves the built web UI from `WEB_DIR`, with `index.html` as the
//!   fallback so client-side routes work on reload. If `WEB_DIR` is missing (e.g. API
//!   only, or the UI isn't built yet) a warning is logged and those paths 404.

mod health;

use axum::Router;
use axum::routing::get;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::errors::ApiError;
pub use crate::state::AppState;

/// Builds the complete application router.
pub fn router(state: AppState) -> Router {
    let api_v1 = Router::new().route("/health", get(health::health));

    let api = Router::new()
        .nest("/v1", api_v1)
        .fallback(|| async { ApiError::NotFound });

    let app = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .nest("/api", api);

    let web_dir = &state.config.web_dir;
    let app = if web_dir.is_dir() {
        let index = web_dir.join("index.html");
        app.fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
    } else {
        tracing::warn!(
            web_dir = %web_dir.display(),
            "WEB_DIR does not exist: the web UI will not be served"
        );
        app.fallback(|| async { ApiError::NotFound })
    };

    app.layer(TraceLayer::new_for_http()).with_state(state)
}

#[cfg(test)]
mod tests;
