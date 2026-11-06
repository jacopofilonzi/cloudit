//! Liveness and readiness probes.

use axum::Json;
use cloudit_core::protocol::HealthResponse;

/// Liveness: the process is up and serving HTTP. Never checks dependencies, so a
/// slow database doesn't get the container restarted.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::ok(env!("CARGO_PKG_VERSION")))
}

/// Readiness: the instance can take traffic. Always ok for now; it will check the
/// database and object storage once they exist (and answer `503 unavailable`).
pub async fn ready() -> Json<HealthResponse> {
    Json(HealthResponse::ok(env!("CARGO_PKG_VERSION")))
}
