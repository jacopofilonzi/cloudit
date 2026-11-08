//! The status of a linked folder.

use serde::Serialize;

use super::plan::{Comparison, PathChange};
use crate::scan::ScanProblem;

/// The state of a linked folder, most important first (when several apply, the first
/// listed wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// The local folder doesn't exist (moved, unmounted drive): relink or recreate it.
    MissingLocalPath,
    /// The server folder is gone (deleted, trashed, access removed).
    RemoteMissing,
    /// Some local items can't be synced (see `problems`), or the last operation failed.
    Error,
    /// Both sides changed since the last sync (or a decision is pending): see the
    /// conflict operations of [`crate::Engine`].
    Conflict,
    /// Only this device changed: push.
    LocalChanges,
    /// Only the server changed: pull.
    RemoteChanges,
    /// In sync, but the server has unresolved conflict snapshots (from any device).
    ServerConflict,
    /// The local content equals the server's head.
    InSync,
}

impl SyncState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingLocalPath => "missing_local_path",
            Self::RemoteMissing => "remote_missing",
            Self::Error => "error",
            Self::Conflict => "conflict",
            Self::LocalChanges => "local_changes",
            Self::RemoteChanges => "remote_changes",
            Self::ServerConflict => "server_conflict",
            Self::InSync => "in_sync",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "missing_local_path" => Self::MissingLocalPath,
            "remote_missing" => Self::RemoteMissing,
            "error" => Self::Error,
            "conflict" => Self::Conflict,
            "local_changes" => Self::LocalChanges,
            "remote_changes" => Self::RemoteChanges,
            "server_conflict" => Self::ServerConflict,
            "in_sync" => Self::InSync,
            _ => return None,
        })
    }
}

/// The computed status of a linked folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderStatus {
    /// Local id of the linked folder.
    pub folder: i64,
    pub state: SyncState,
    /// No automatic sync (a decision was postponed).
    pub paused: bool,
    /// Paths only this device changed.
    pub local_changes: Vec<PathChange>,
    /// Paths only the server changed.
    pub remote_changes: Vec<PathChange>,
    /// Paths both changed.
    pub conflicting: Vec<String>,
    /// Items that can't be synced.
    pub problems: Vec<ScanProblem>,
    pub base_snapshot_id: Option<String>,
    pub head_snapshot_id: Option<String>,
    /// Unresolved conflict snapshots on the server (any device).
    pub unresolved_conflicts: u64,
    /// The conflict snapshot this device uploaded, waiting for a decision.
    pub pending_conflict_id: Option<String>,
}

impl FolderStatus {
    /// A status with only a state (nothing could be compared).
    #[must_use]
    pub fn bare(folder: i64, state: SyncState, paused: bool) -> Self {
        Self {
            folder,
            state,
            paused,
            local_changes: Vec::new(),
            remote_changes: Vec::new(),
            conflicting: Vec::new(),
            problems: Vec::new(),
            base_snapshot_id: None,
            head_snapshot_id: None,
            unresolved_conflicts: 0,
            pending_conflict_id: None,
        }
    }
}

/// The state from a comparison (see [`SyncState`] for the precedence).
#[must_use]
pub fn state_of(
    comparison: &Comparison,
    has_problems: bool,
    pending_conflict: bool,
    unresolved_conflicts: u64,
) -> SyncState {
    if has_problems {
        SyncState::Error
    } else if comparison.is_conflict() || pending_conflict {
        SyncState::Conflict
    } else if !comparison.local.is_empty() {
        SyncState::LocalChanges
    } else if !comparison.remote.is_empty() {
        SyncState::RemoteChanges
    } else if unresolved_conflicts > 0 {
        SyncState::ServerConflict
    } else {
        SyncState::InSync
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::plan::ChangeKind;

    #[test]
    fn states_round_trip_and_precedence() {
        for state in [
            SyncState::MissingLocalPath,
            SyncState::RemoteMissing,
            SyncState::Error,
            SyncState::Conflict,
            SyncState::LocalChanges,
            SyncState::RemoteChanges,
            SyncState::ServerConflict,
            SyncState::InSync,
        ] {
            assert_eq!(SyncState::parse(state.as_str()), Some(state));
        }
        let change = PathChange {
            path: "a".to_owned(),
            change: ChangeKind::Added,
        };
        let local = Comparison {
            local: vec![change.clone()],
            ..Comparison::default()
        };
        let both = Comparison {
            local: vec![change.clone()],
            remote: vec![change],
            ..Comparison::default()
        };
        let none = Comparison::default();
        assert_eq!(state_of(&local, true, false, 0), SyncState::Error);
        assert_eq!(state_of(&both, false, false, 0), SyncState::Conflict);
        assert_eq!(state_of(&none, false, true, 0), SyncState::Conflict);
        assert_eq!(state_of(&local, false, false, 3), SyncState::LocalChanges);
        assert_eq!(state_of(&none, false, false, 3), SyncState::ServerConflict);
        assert_eq!(state_of(&none, false, false, 0), SyncState::InSync);
    }
}
