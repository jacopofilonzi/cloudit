//! Storage usage and quotas (`/api/v1/me/usage`).
//!
//! Usage counts the stored (encoded, deduplicated per scope) bytes of the chunks and
//! manifests of every scope a user **owns**, trashed scopes included; uploads by the
//! members of a shared scope count against its owner. Space is freed only when the
//! garbage collector deletes objects no snapshot references any more.
//!
//! The effective quota is the user's own (`quota_bytes`, set by an admin) or else the
//! server default (`default_quota_bytes`); `null` means unlimited. An upload that would
//! exceed it fails with `quota_exceeded` (507).

use serde::{Deserialize, Serialize};

/// One owned scope in [`UsageInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ScopeUsage {
    pub id: String,
    pub name: String,
    pub stored_bytes: u64,
    /// In the trash (still counted until deleted for good).
    pub trashed: bool,
}

/// `GET /api/v1/me/usage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UsageInfo {
    /// Sum of `stored_bytes` over the scopes the caller owns.
    pub used_bytes: u64,
    /// The effective quota; `null` = unlimited.
    pub quota_bytes: Option<u64>,
    /// The owned scopes, by name.
    pub scopes: Vec<ScopeUsage>,
}
