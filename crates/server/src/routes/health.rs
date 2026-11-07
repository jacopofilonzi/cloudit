//! Liveness and readiness probes.

use std::future::Future;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use cloudit_core::protocol::{
    ComponentStatus, HealthResponse, ReadyComponents, ReadyResponse, ReadyStatus,
};

use crate::state::AppState;

/// Upper bound for each readiness check, so a hung dependency still gets an answer.
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Liveness: the process is up and serving HTTP. Never checks dependencies, so a
/// slow database doesn't get the container restarted.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::ok(env!("CARGO_PKG_VERSION")))
}

async fn check<F>(component: &'static str, fut: F) -> ComponentStatus
where
    F: Future<Output = anyhow::Result<()>>,
{
    match tokio::time::timeout(CHECK_TIMEOUT, fut).await {
        Ok(Ok(())) => ComponentStatus::Ok,
        Ok(Err(err)) => {
            tracing::warn!(component, error = %format!("{err:#}"), "readiness check failed");
            ComponentStatus::Unavailable
        }
        Err(_) => {
            tracing::warn!(component, "readiness check timed out");
            ComponentStatus::Unavailable
        }
    }
}

/// Readiness: the instance can take traffic. 200 only if the database and blob
/// storage answer; Redis is reported but never makes it fail (it is optional).
pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadyResponse>) {
    let redis = async {
        if state.config.redis_url.is_none() {
            return ComponentStatus::Disabled;
        }
        match &state.redis {
            Some(redis) if redis.ping().await => ComponentStatus::Ok,
            _ => ComponentStatus::Unavailable,
        }
    };
    let (database, storage, redis) = tokio::join!(
        check("database", crate::db::ping(&state.db)),
        check("storage", state.blobs.check()),
        redis
    );
    let ok = database == ComponentStatus::Ok && storage == ComponentStatus::Ok;
    let body = ReadyResponse {
        status: if ok {
            ReadyStatus::Ok
        } else {
            ReadyStatus::Unavailable
        },
        version: env!("CARGO_PKG_VERSION").to_owned(),
        components: ReadyComponents {
            database,
            storage,
            redis,
        },
    };
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body))
}
