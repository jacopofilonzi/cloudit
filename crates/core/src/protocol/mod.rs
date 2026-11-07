//! HTTP API types shared by the server, the desktop client and (via `ts-rs`) the web UI.
//!
//! Types derive `ts_rs::TS` only under `cfg(test)`: the export happens in the
//! generated `export_bindings_*` tests, so `ts-rs` stays a dev-dependency and never
//! ships in the server binary. Run `make types` after changing anything here.
//!
//! Conventions: ids are UUID strings; timestamps are RFC 3339 UTC strings
//! (`2026-01-31T12:00:00Z`); request bodies reject unknown fields; optional request
//! fields may be omitted; secret values (passwords, tokens) are [`Sensitive`], which
//! never prints its content in `Debug` output.
//!
//! - this file: health and readiness probes, error codes and the error body;
//! - [`auth`]: first-run setup, registration, login, password reset;
//! - [`me`]: the signed-in user's profile, password and sessions;
//! - [`devices`]: desktop client login (loopback + PKCE) and device management;
//! - [`admin`]: user management, invites, password resets and server settings;
//! - [`providers`]: identity providers (presets, admin configuration);
//! - [`oauth`]: sign-in through providers and linked identities.

mod admin;
mod auth;
mod devices;
mod me;
mod oauth;
mod providers;
mod sensitive;

pub use admin::*;
pub use auth::*;
pub use devices::*;
pub use me::*;
pub use oauth::*;
pub use providers::*;
pub use sensitive::Sensitive;

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
    /// Not signed in, or the session / device token is invalid or expired (401).
    Unauthorized,
    /// Signed in, but not allowed (admin only, or a device token on a session-only
    /// route) (403).
    Forbidden,
    /// Wrong email or password; deliberately the same for both (401).
    InvalidCredentials,
    /// The account is disabled by an admin (403).
    AccountDisabled,
    /// Registration mode is `closed` (403).
    RegistrationClosed,
    /// The invite is unknown, used, expired, revoked, missing (mode `invite`) or bound
    /// to another email (400).
    InviteInvalid,
    /// A password-reset token, device authorization code or PKCE verifier is invalid,
    /// used or expired (400).
    TokenInvalid,
    /// Another account already uses this email (409).
    EmailTaken,
    /// A field fails validation; `field` in the error body names it (422).
    ValidationFailed,
    /// Too many attempts; retry after the `Retry-After` header (429).
    RateLimited,
    /// Cross-site request refused: `Origin` doesn't match the server (403).
    CsrfFailed,
    /// First-run setup is not done yet: only setup, health and `auth/config` work (409).
    SetupRequired,
    /// First-run setup was already completed (409).
    SetupAlreadyDone,
    /// Wrong setup token (403).
    SetupTokenInvalid,
    /// The change would leave no enabled admin (409).
    LastAdmin,
    /// No identity provider with this id (or it was deleted) (404).
    ProviderNotFound,
    /// A provider with this id exists or existed (ids are never reused) (409).
    ProviderIdTaken,
    /// The provider's OIDC discovery document or keys could not be fetched or are
    /// invalid (502).
    ProviderDiscoveryFailed,
    /// The identity provider failed or answered something unusable (token exchange,
    /// ID token or user info); details are only in the server logs (502).
    ProviderError,
    /// Provider sign-in needs the server's public URL (setting or `PUBLIC_URL`) to
    /// build the callback URL (409).
    PublicUrlRequired,
    /// The identity provider is disabled by an admin (403).
    ProviderDisabled,
    /// The provider sign-in can't be completed: unknown, expired or already used
    /// `state`, or started in another browser (400).
    OauthStateInvalid,
    /// The sign-in was cancelled or refused at the provider (400).
    OauthCancelled,
    /// The provider account is not in any of the provider's allowed groups (403).
    GroupNotAllowed,
    /// This provider account is already linked to another cloudit account (409).
    IdentityInUse,
    /// This provider account is not linked to any cloudit account (409).
    IdentityNotLinked,
    /// The pending provider sign-up is missing, expired or already used: sign in
    /// again (410).
    OnboardingExpired,
}

/// JSON body of every API error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ApiErrorBody {
    pub code: ErrorCode,
    /// English description for logs and debugging; never shown to users as-is.
    pub message: String,
    /// With `validation_failed`: the request field at fault (e.g. `email`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub field: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_snake_case() {
        let body = ApiErrorBody {
            code: ErrorCode::NotFound,
            message: "no route".to_owned(),
            field: None,
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
        let body = ApiErrorBody {
            code: ErrorCode::ValidationFailed,
            message: "bad email".to_owned(),
            field: Some("email".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({ "code": "validation_failed", "message": "bad email", "field": "email" })
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::SetupAlreadyDone).unwrap(),
            serde_json::json!("setup_already_done")
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
