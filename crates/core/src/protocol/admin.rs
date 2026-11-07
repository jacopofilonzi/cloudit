//! Administration (`/api/v1/admin/*`, web session of an admin only).

use serde::{Deserialize, Serialize};

use super::{RegistrationMode, Sensitive, TokenTtl};

/// One account as seen by admins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AdminUser {
    pub id: String,
    pub email: String,
    pub email_verified: bool,
    pub display_name: String,
    /// Effective admin: `admin || admin_from_groups`.
    pub is_admin: bool,
    /// Manual admin flag (editable here).
    pub admin: bool,
    /// Admin granted by identity-provider groups (milestone 3c; read-only).
    pub admin_from_groups: bool,
    pub disabled_at: Option<String>,
    pub has_password: bool,
    pub created_at: String,
}

/// `GET /api/v1/admin/users?limit=&offset=` (oldest first; limit 1..=200, default 50).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AdminUserList {
    pub users: Vec<AdminUser>,
    /// Number of accounts in total.
    pub total: u64,
}

/// `PATCH /api/v1/admin/users/{id}`: omitted fields stay unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AdminUserUpdate {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub admin: Option<bool>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub disabled: Option<bool>,
}

/// A single-use password-reset link (valid 24 hours). The UI builds the full URL from
/// `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct PasswordResetLink {
    pub token: Sensitive,
    /// `/reset-password/<token>`.
    pub path: String,
    pub expires_at: String,
}

/// `POST /api/v1/admin/invites`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CreateInviteRequest {
    /// If set, registration must use this email, which then counts as verified.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub email: Option<String>,
    /// Default `7d`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub expires_in: Option<TokenTtl>,
}

/// A new invite: the token is shown once, only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct InviteLink {
    pub id: String,
    pub token: Sensitive,
    /// `/invite/<token>`.
    pub path: String,
    pub expires_at: String,
}

/// State of an invite, derived from its timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum InviteStatus {
    Pending,
    Used,
    Expired,
    Revoked,
}

/// One invite in `GET /api/v1/admin/invites` (newest first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct InviteInfo {
    pub id: String,
    pub email: Option<String>,
    pub status: InviteStatus,
    pub created_by: String,
    pub created_at: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_by: Option<String>,
    pub revoked_at: Option<String>,
}

/// `GET /api/v1/admin/settings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ServerSettings {
    pub registration_mode: RegistrationMode,
    /// Sign in an unknown identity-provider account into the existing account with the
    /// same email, when the provider says the email is verified and the existing
    /// account's email is verified too. Default `false`.
    pub autolink_email: bool,
    /// Public URL stored in the database (`http(s)://host[:port]`), if any.
    pub public_url: Option<String>,
    /// `true` when the `PUBLIC_URL` environment variable is set: it wins over
    /// `public_url`.
    pub public_url_from_env: bool,
    /// The URL in use (environment, else stored): identity-provider callback URLs are
    /// built from it.
    pub effective_public_url: Option<String>,
}

/// `PATCH /api/v1/admin/settings`: omitted fields stay unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub registration_mode: Option<RegistrationMode>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub autolink_email: Option<bool>,
    /// `http(s)://host[:port]` (a trailing `/` is removed; no path, query or fragment);
    /// an empty string clears it.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub public_url: Option<String>,
}
