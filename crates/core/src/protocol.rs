//! HTTP API types shared by the server, the desktop client and (via `ts-rs`) the web UI.
//!
//! Types derive `ts_rs::TS` only under `cfg(test)`: the export happens in the
//! generated `export_bindings_*` tests, so `ts-rs` stays a dev-dependency and never
//! ships in the server binary. Run `make types` after changing anything here.

use serde::{Deserialize, Serialize};

/// Liveness payload returned by `GET /health` and `GET /api/v1/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct HealthResponse {
    /// `"ok"` when the server can serve requests.
    pub status: String,
    /// Server version (`x.y.z`), from the workspace `Cargo.toml`.
    pub version: String,
}

impl HealthResponse {
    /// A healthy response carrying the given version.
    #[must_use]
    pub fn ok(version: impl Into<String>) -> Self {
        Self {
            status: "ok".to_owned(),
            version: version.into(),
        }
    }
}

/// Overall readiness in [`ReadyResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ReadyStatus {
    /// Every required component works: the instance can take traffic (HTTP 200).
    Ok,
    /// A required component (database or storage) is failing (HTTP 503).
    Unavailable,
}

/// State of one dependency in [`ReadyComponents`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ComponentStatus {
    /// Reachable and working.
    Ok,
    /// Configured but failing its check.
    Unavailable,
    /// Not configured (only optional components, i.e. Redis).
    Disabled,
}

/// Per-dependency readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ReadyComponents {
    /// Database (`SELECT 1`). Required.
    pub database: ComponentStatus,
    /// Blob storage (filesystem writable / bucket listable). Required.
    pub storage: ComponentStatus,
    /// Redis: optional, never makes the instance unready.
    pub redis: ComponentStatus,
}

/// Payload of `GET /ready`: HTTP 200 when `status` is `ok`, else 503.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ReadyResponse {
    pub status: ReadyStatus,
    /// Server version (`x.y.z`).
    pub version: String,
    pub components: ReadyComponents,
}

/// Stable, machine-readable error codes. The UI maps each code to its own
/// translated text; the `message` next to it is English, for logs only.
/// Never rename or reuse a code: clients match on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ErrorCode {
    /// The request is malformed or fails validation.
    BadRequest,
    /// No such route or resource.
    NotFound,
    /// A dependency needed to serve the request is unavailable; retry later.
    Unavailable,
    /// Unexpected server-side failure.
    Internal,
}

/// JSON body of every API error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ApiErrorBody {
    pub code: ErrorCode,
    /// English description for logs and debugging; never shown to users as-is.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_snake_case() {
        let body = ApiErrorBody {
            code: ErrorCode::NotFound,
            message: "no route".to_owned(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "not_found", "message": "no route" })
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::BadRequest).unwrap(),
            serde_json::json!("bad_request")
        );
    }

    #[test]
    fn health_response_json_shape() {
        let json = serde_json::to_value(HealthResponse::ok("1.2.3")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "status": "ok", "version": "1.2.3" })
        );
    }

    #[test]
    fn ready_response_json_shape() {
        let ready = ReadyResponse {
            status: ReadyStatus::Unavailable,
            version: "1.2.3".to_owned(),
            components: ReadyComponents {
                database: ComponentStatus::Ok,
                storage: ComponentStatus::Unavailable,
                redis: ComponentStatus::Disabled,
            },
        };
        assert_eq!(
            serde_json::to_value(&ready).unwrap(),
            serde_json::json!({
                "status": "unavailable",
                "version": "1.2.3",
                "components": { "database": "ok", "storage": "unavailable", "redis": "disabled" }
            })
        );
    }

    #[test]
    fn health_response_round_trips() {
        let original = HealthResponse::ok("0.1.0");
        let text = serde_json::to_string(&original).unwrap();
        let parsed: HealthResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, original);
    }
}
