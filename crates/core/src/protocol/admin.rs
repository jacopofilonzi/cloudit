//! Administration (`/api/v1/admin/*`, web session of an admin only).

use serde::{Deserialize, Serialize};

use super::{
    ApprovalStatus, JobOutcome, NewAccounts, Patch, RegistrationMode, Sensitive, TokenTtl,
};

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
    /// Ids of the providers this account has identities at (sorted; deleted
    /// providers included).
    pub identity_providers: Vec<String>,
    pub approval_status: ApprovalStatus,
    /// How the account was created: `password` (registration without an invite),
    /// `invite` (any sign-up that consumed an invite), or the id of the identity
    /// provider it signed up with; `null` for the first admin and for accounts created
    /// before approvals existed.
    pub signup_source: Option<String>,
    /// When an admin last approved or rejected the account (`null` if never: approved
    /// automatically or still pending).
    pub approval_decided_at: Option<String>,
    /// The admin who made that decision.
    pub approval_decided_by: Option<String>,
    pub created_at: String,
    /// Stored bytes of the scopes the account owns (trashed ones included).
    pub used_bytes: u64,
    /// The account's own quota; `null` = the server default (or unlimited, see
    /// `quota_unlimited`).
    pub quota_bytes: Option<u64>,
    /// The account has no quota, whatever the server default (`quota_bytes` is then
    /// `null`).
    pub quota_unlimited: bool,
    /// The quota in force (none if unlimited, else own, else server default); `null` =
    /// unlimited.
    pub effective_quota_bytes: Option<u64>,
}

/// `GET /api/v1/admin/users?limit=&offset=&approval_status=` (oldest first; limit
/// 1..=200, default 50; `approval_status` filters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AdminUserList {
    pub users: Vec<AdminUser>,
    /// Number of accounts matching the filter.
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
    /// Storage quota in bytes. Omitted: unchanged; `null`: the server default. Sent
    /// (`null` included) without `quota_unlimited: true`, it clears `quota_unlimited`.
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    #[cfg_attr(test, ts(as = "Option<Option<u64>>", optional))]
    pub quota_bytes: Patch<u64>,
    /// `true`: no quota at all (clears `quota_bytes`); `false`: not unlimited (an
    /// unlimited account goes back to the server default unless `quota_bytes` is
    /// sent). `true` with a number in `quota_bytes` is refused (`validation_failed`,
    /// field `quota_unlimited`).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub quota_unlimited: Option<bool>,
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
    /// Whether password registrations without an invite need an admin's approval.
    /// Default `approval`.
    pub password_new_accounts: NewAccounts,
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
    /// Default of a folder's `retention_max_versions`: snapshots kept per folder
    /// (0 = unlimited). Default 20.
    pub retention_max_versions: u32,
    /// Default of a folder's `retention_max_per_day`: snapshots kept per UTC day
    /// (0 = unlimited). Default 5.
    pub retention_max_per_day: u32,
    /// Days an item stays in the trash before it is deleted for good. Default 30.
    pub trash_retention_days: u32,
    /// Storage quota of accounts without their own, in bytes; `null` = unlimited
    /// (the default).
    pub default_quota_bytes: Option<u64>,
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
    pub password_new_accounts: Option<NewAccounts>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub autolink_email: Option<bool>,
    /// `http(s)://host[:port]` (a trailing `/` is removed; no path, query or fragment);
    /// an empty string clears it.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub public_url: Option<String>,
    /// 0 (unlimited) to 100000.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub retention_max_versions: Option<u32>,
    /// 0 (unlimited) to 100000.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub retention_max_per_day: Option<u32>,
    /// 1 to 3650.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub trash_retention_days: Option<u32>,
    /// Omitted: unchanged; `null`: unlimited; a number of bytes.
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    #[cfg_attr(test, ts(as = "Option<Option<u64>>", optional))]
    pub default_quota_bytes: Patch<u64>,
}

/// `GET /api/v1/admin/summary`: counters for the admin UI (badges).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AdminSummary {
    /// Accounts waiting for approval.
    pub pending_users: u64,
    /// The version this server runs.
    pub server_version: String,
    /// The latest published server version, as the `update_check` job last saw it
    /// (`null`: never checked, checks disabled, or no server version yet).
    pub latest_release: Option<LatestRelease>,
    /// `latest_release` is newer than `server_version`.
    pub update_available: bool,
    /// Where the update check stands (reading the summary may request one).
    pub update_check: UpdateCheckStatus,
}

/// The `update_check` job as the admin UI needs it (`AdminSummary`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateCheckStatus {
    /// `false` with `UPDATE_CHECK=false`: the server never checks.
    pub enabled: bool,
    /// A check is requested or running: read the summary again shortly.
    pub pending: bool,
    /// How the last finished check ended (`null`: none yet).
    pub last_outcome: Option<JobOutcome>,
    /// When it ended (RFC 3339).
    pub last_finished_at: Option<String>,
}

/// The latest published cloudit server version (`update_check` job: the highest
/// `server-vX.Y.Z` tag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct LatestRelease {
    /// `major.minor.patch`, without the tag's `v`.
    pub version: String,
    /// Its release notes (`https://…`: the server changelog).
    pub url: String,
    /// When the job fetched it (RFC 3339).
    pub checked_at: String,
}
