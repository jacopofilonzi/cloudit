//! Snapshots: the immutable versions of a folder (`/api/v1/folders/{id}/snapshots`,
//! `/api/v1/snapshots/{id}*`).
//!
//! A commit uploads a manifest (`manifest::codec` stored form) whose chunks the scope
//! already has, and moves the folder's head from `parent` to the new snapshot only if
//! the head still is `parent` (else `conflict`, 409, with the current head). A client
//! that lost that race keeps its local state as a `conflict` snapshot (which never
//! moves the head) until the user decides: keep the server version (`resolve`), keep
//! the local one (`restore` the conflict snapshot) or decide later.

use serde::{Deserialize, Serialize};

use super::Patch;
use crate::hash::ManifestHash;

/// Kind of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum SnapshotKind {
    /// A regular upload; became the head when committed.
    Normal,
    /// A client's local state that lost a commit race; never the head by itself.
    Conflict,
    /// A copy of an older snapshot made the head again.
    Restore,
}

impl SnapshotKind {
    /// The stored and API name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Conflict => "conflict",
            Self::Restore => "restore",
        }
    }

    /// Parses a stored or API name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "conflict" => Some(Self::Conflict),
            "restore" => Some(Self::Restore),
            _ => None,
        }
    }
}

/// One snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SnapshotInfo {
    pub id: String,
    pub folder_id: String,
    /// The head the snapshot was based on (`null` for the first one). For a conflict:
    /// the base the client had.
    pub parent_id: Option<String>,
    pub kind: SnapshotKind,
    pub manifest_hash: ManifestHash,
    /// Number of files (empty directories not counted).
    pub file_count: u64,
    /// Sum of the file sizes, in bytes.
    pub logical_size: u64,
    /// The user who uploaded or restored it.
    pub author_id: String,
    /// The device, when it was made with a device token.
    pub device_id: Option<String>,
    pub created_at: String,
    /// Pinned snapshots are never removed by retention.
    pub pinned: bool,
    pub label: Option<String>,
    /// For `restore`: the snapshot it copies.
    pub restored_from: Option<String>,
    /// For `conflict`: when the user resolved it (`null` = still open).
    pub conflict_resolved_at: Option<String>,
    /// Whether this is the folder's current head.
    pub is_head: bool,
}

/// Answer of a commit: 201 with the new snapshot, or 200 with `unchanged: true` and the
/// current head when the manifest equals the head's (nothing was created).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CommitResponse {
    pub unchanged: bool,
    pub snapshot: SnapshotInfo,
}

/// A page of `GET /api/v1/folders/{id}/snapshots` (newest first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SnapshotList {
    pub snapshots: Vec<SnapshotInfo>,
    /// Pass as `cursor` for the next (older) page; `null` on the last page.
    pub next_cursor: Option<String>,
}

/// `PATCH /api/v1/snapshots/{id}` (editor or more): omitted fields stay unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateSnapshotRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub pinned: Option<bool>,
    /// Trimmed, 1 to 100 characters, no control characters; `null` or `""` clears it.
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    #[cfg_attr(test, ts(as = "Option<Option<String>>", optional))]
    pub label: Patch<String>,
}

/// `POST /api/v1/snapshots/{id}/restore` (editor or more).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct RestoreSnapshotRequest {
    /// The head the client believes current: if the head moved, `conflict` (409).
    /// Omitted: restore on top of whatever the head is.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub expected_head: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in [
            SnapshotKind::Normal,
            SnapshotKind::Conflict,
            SnapshotKind::Restore,
        ] {
            assert_eq!(SnapshotKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
        assert_eq!(SnapshotKind::parse("other"), None);
    }

    #[test]
    fn label_patch_distinguishes_null() {
        let body: UpdateSnapshotRequest = serde_json::from_str(r#"{"label":null}"#).unwrap();
        assert_eq!(body.label, Patch::Null);
        let body: UpdateSnapshotRequest = serde_json::from_str("{}").unwrap();
        assert!(body.label.is_missing());
        assert!(serde_json::from_str::<UpdateSnapshotRequest>(r#"{"x":1}"#).is_err());
    }
}
