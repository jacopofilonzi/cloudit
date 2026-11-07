//! The signed-in user (`/api/v1/me`).

use serde::{Deserialize, Serialize};

use super::Sensitive;

/// `GET /api/v1/me`; also returned by setup, register and login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UserProfile {
    pub id: String,
    /// Trimmed and lowercased.
    pub email: String,
    /// `true` only when the email came from an invite bound to it, the first-run setup
    /// or (later) a trusted identity provider; self-entered emails are unverified.
    pub email_verified: bool,
    pub display_name: String,
    /// Effective admin: manual flag or (later) granted by identity-provider groups.
    pub is_admin: bool,
    /// `false` for accounts that only sign in through an identity provider.
    pub has_password: bool,
    pub created_at: String,
}

/// `PATCH /api/v1/me`: omitted fields stay unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateMeRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub display_name: Option<String>,
    /// A new email becomes unverified.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub email: Option<String>,
    /// Required to change the email when the account has a password.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub current_password: Option<Sensitive>,
}

/// `POST /api/v1/me/password`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChangePasswordRequest {
    /// Required when the account already has a password.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub current_password: Option<Sensitive>,
    pub new_password: Sensitive,
}

/// One web session in `GET /api/v1/me/sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SessionInfo {
    pub id: String,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub user_agent: Option<String>,
    /// The session making this request.
    pub current: bool,
}
