//! HTTP API types shared by the server, the desktop client and (via `ts-rs`) the web UI.
//!
//! Types derive `ts_rs::TS` only under `cfg(test)`: the export happens in the
//! generated `export_bindings_*` tests, so `ts-rs` stays a dev-dependency and never
//! ships in the server binary. Run `make types` after changing anything here.

use serde::{Deserialize, Serialize};

/// Liveness/readiness payload returned by `GET /health`, `GET /ready` and `GET /api/v1/health`.
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
    fn health_response_round_trips() {
        let original = HealthResponse::ok("0.1.0");
        let text = serde_json::to_string(&original).unwrap();
        let parsed: HealthResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, original);
    }
}
