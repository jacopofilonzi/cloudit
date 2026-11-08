//! Desktop client login and device management (`/api/v1/devices`).
//!
//! Login follows RFC 8252 (native apps, loopback redirect) with PKCE S256 (RFC 7636):
//! the client opens the web UI, which calls `authorize` with the user's session and
//! sends the browser to `redirect_url` (`http://127.0.0.1:<port>/<path>?code=…&state=…`);
//! the client then exchanges the code and its verifier at `token` for a device token
//! (`cld_…`), sent afterwards as `Authorization: Bearer cld_…`.

use serde::{Deserialize, Serialize};

use super::Sensitive;

/// Lifetime of a device token (fixed, not extended by use), also used for invites
/// (which refuse `forever`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum TokenTtl {
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "1d")]
    OneDay,
    #[serde(rename = "7d")]
    SevenDays,
    #[default]
    #[serde(rename = "30d")]
    ThirtyDays,
    #[serde(rename = "1y")]
    OneYear,
    /// Never expires: valid until revoked (or the account is disabled or rejected).
    #[serde(rename = "forever")]
    Forever,
}

impl TokenTtl {
    /// The lifetime in seconds (a year is 365 days); `None` = forever.
    #[must_use]
    pub fn as_secs(self) -> Option<u64> {
        const DAY: u64 = 86_400;
        match self {
            Self::OneHour => Some(3600),
            Self::OneDay => Some(DAY),
            Self::SevenDays => Some(7 * DAY),
            Self::ThirtyDays => Some(30 * DAY),
            Self::OneYear => Some(365 * DAY),
            Self::Forever => None,
        }
    }
}

/// PKCE method; only `S256` is accepted (`plain` is not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum CodeChallengeMethod {
    S256,
}

/// `POST /api/v1/devices/authorize` (web session only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct DeviceAuthorizeRequest {
    /// base64url (no padding) of SHA-256(code_verifier): 43 characters.
    pub code_challenge: String,
    pub code_challenge_method: CodeChallengeMethod,
    /// `http://127.0.0.1:<port>/<path>` or `http://[::1]:<port>/<path>`; no query,
    /// fragment or `localhost`.
    pub redirect_uri: String,
    /// Opaque client value echoed in the redirect (1 to 512 characters).
    pub state: String,
    pub device_name: String,
    /// Client OS, free text (e.g. `windows`, `linux`).
    pub os: String,
    /// Default `30d`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub token_ttl: Option<TokenTtl>,
}

/// Answer to `authorize`: where to send the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct DeviceAuthorizeResponse {
    pub redirect_url: String,
}

/// `POST /api/v1/devices/token` (no authentication; the code is single use and lives
/// two minutes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct DeviceTokenRequest {
    pub code: Sensitive,
    pub code_verifier: Sensitive,
    /// Exactly the `redirect_uri` given to `authorize`.
    pub redirect_uri: String,
}

/// The new device token; shown once, only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct DeviceTokenResponse {
    /// `cld_…`, for `Authorization: Bearer`.
    pub token: Sensitive,
    pub device_id: String,
    /// `null` for a `forever` token.
    pub expires_at: Option<String>,
}

/// One device in `GET /api/v1/devices` (revoked and expired ones included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub os: String,
    pub created_at: String,
    /// Last authenticated request, updated at most every few minutes.
    pub last_seen_at: Option<String>,
    /// `null` = never expires.
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// `PATCH /api/v1/devices/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateDeviceRequest {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_values() {
        assert_eq!(
            serde_json::to_value(TokenTtl::SevenDays).unwrap(),
            serde_json::json!("7d")
        );
        let ttl: TokenTtl = serde_json::from_value(serde_json::json!("1y")).unwrap();
        assert_eq!(ttl.as_secs(), Some(365 * 86_400));
        assert_eq!(TokenTtl::default(), TokenTtl::ThirtyDays);
        assert_eq!(
            serde_json::to_value(TokenTtl::Forever).unwrap(),
            serde_json::json!("forever")
        );
        assert_eq!(TokenTtl::Forever.as_secs(), None);
        assert!(serde_json::from_value::<TokenTtl>(serde_json::json!("2d")).is_err());
        assert!(serde_json::from_value::<CodeChallengeMethod>(serde_json::json!("plain")).is_err());
    }
}
