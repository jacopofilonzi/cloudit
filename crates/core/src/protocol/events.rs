//! Real-time events (`GET /api/v1/events`, Server-Sent Events) and the polling
//! fallback (`GET /api/v1/sync/heads`).
//!
//! Each SSE message has `event: <type>` (the snake_case name of the [`ServerEvent`]
//! variant) and `data: <JSON payload>` (the variant's struct). A stream starts with a
//! `hello`; comment lines (`: keep-alive`) arrive about every 25 s. A new `hello` is
//! sent when the server's `realtime` mode changes.
//!
//! Only users with access to a scope (its owner and members at the time of the change,
//! plus the affected user of a membership change) receive its events.
//!
//! **Client contract**: on every (re)connect, fetch `GET /sync/heads` first, then apply
//! the events. When `hello.realtime` is `local`, events may not cover changes made
//! through other server instances: also poll `/sync/heads` periodically (e.g. every
//! 5 minutes; `If-None-Match` makes an unchanged answer a 304). A stream that ends (the
//! credentials were revoked, the client fell behind, the server restarted) means
//! reconnect and resync.

use serde::{Deserialize, Serialize};

/// Whether events reach clients of every server instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum RealtimeMode {
    /// Events cross instances (Redis pub/sub works).
    Shared,
    /// Only changes made through this instance are delivered: poll `/sync/heads` too.
    Local,
}

/// First message of every stream (and sent again when `realtime` changes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct HelloEvent {
    pub server_time: String,
    /// The server instance holding the stream.
    pub instance_id: String,
    pub realtime: RealtimeMode,
}

/// A folder's head moved (commit or restore).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct FolderHeadEvent {
    pub scope_id: String,
    pub folder_id: String,
    /// The new head.
    pub snapshot_id: String,
}

/// A `conflict` snapshot was committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ConflictEvent {
    pub scope_id: String,
    pub folder_id: String,
    pub snapshot_id: String,
}

/// A folder's history changed without a new head: pin, label, a conflict resolved,
/// snapshots deleted by retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct HistoryChangedEvent {
    pub scope_id: String,
    pub folder_id: String,
}

/// What happened to a scope or folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ChangeKind {
    Created,
    /// Renamed, or a folder's retention changed.
    Updated,
    Trashed,
    Restored,
    /// Deleted for good (trash purge).
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct FolderChangedEvent {
    pub scope_id: String,
    pub folder_id: String,
    pub change: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ScopeChangedEvent {
    pub scope_id: String,
    pub change: ChangeKind,
}

/// A member was added, changed role, was removed or left (sent to every member and to
/// the affected user).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct MembershipChangedEvent {
    pub scope_id: String,
}

/// The recipient's storage usage went down (garbage collection, trash purge): refetch
/// `/me/usage` if shown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UsageChangedEvent {}

/// One event: `type` is the SSE `event:` name, `data` the JSON of the SSE `data:` line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ServerEvent {
    Hello(HelloEvent),
    FolderHead(FolderHeadEvent),
    Conflict(ConflictEvent),
    HistoryChanged(HistoryChangedEvent),
    FolderChanged(FolderChangedEvent),
    ScopeChanged(ScopeChangedEvent),
    MembershipChanged(MembershipChangedEvent),
    UsageChanged(UsageChangedEvent),
}

impl ServerEvent {
    /// The SSE `event:` name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::FolderHead(_) => "folder_head",
            Self::Conflict(_) => "conflict",
            Self::HistoryChanged(_) => "history_changed",
            Self::FolderChanged(_) => "folder_changed",
            Self::ScopeChanged(_) => "scope_changed",
            Self::MembershipChanged(_) => "membership_changed",
            Self::UsageChanged(_) => "usage_changed",
        }
    }

    /// The SSE `data:` JSON (the payload only).
    pub fn data_json(&self) -> serde_json::Result<String> {
        match self {
            Self::Hello(e) => serde_json::to_string(e),
            Self::FolderHead(e) => serde_json::to_string(e),
            Self::Conflict(e) => serde_json::to_string(e),
            Self::HistoryChanged(e) => serde_json::to_string(e),
            Self::FolderChanged(e) => serde_json::to_string(e),
            Self::ScopeChanged(e) => serde_json::to_string(e),
            Self::MembershipChanged(e) => serde_json::to_string(e),
            Self::UsageChanged(e) => serde_json::to_string(e),
        }
    }
}

/// The sync state of one folder in [`SyncHeads`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SyncHead {
    pub scope_id: String,
    pub folder_id: String,
    pub head_snapshot_id: Option<String>,
    pub head_created_at: Option<String>,
    pub unresolved_conflicts: u64,
}

/// `GET /api/v1/sync/heads`: every folder the caller can see (outside the trash), by
/// folder id. Answered with an `ETag`; `If-None-Match` → 304 when nothing changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SyncHeads {
    pub folders: Vec<SyncHead>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_serde_tag() {
        let events = [
            ServerEvent::Hello(HelloEvent {
                server_time: "2026-01-01T00:00:00Z".to_owned(),
                instance_id: "i".to_owned(),
                realtime: RealtimeMode::Local,
            }),
            ServerEvent::FolderChanged(FolderChangedEvent {
                scope_id: "s".to_owned(),
                folder_id: "f".to_owned(),
                change: ChangeKind::Trashed,
            }),
            ServerEvent::UsageChanged(UsageChangedEvent {}),
        ];
        for event in events {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["type"], event.name());
            let data: serde_json::Value =
                serde_json::from_str(&event.data_json().unwrap()).unwrap();
            assert_eq!(value["data"], data);
            let back: ServerEvent = serde_json::from_value(value).unwrap();
            assert_eq!(back, event);
        }
    }
}
