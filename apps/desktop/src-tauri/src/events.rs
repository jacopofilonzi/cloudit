//! Events the backend pushes to the UI (Tauri events), and the payloads of the ones
//! that aren't engine types. Names are mirrored in `apps/desktop/src/lib/events.ts`.

use cloudit_client::{Account, FolderStatus, SyncReport};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::CommandError;

/// `Progress` of a running operation (throttled).
pub const FOLDER_PROGRESS: &str = "folder-progress";
/// A folder's computed status (`FolderStatusEvent`).
pub const FOLDER_STATUS: &str = "folder-status";
/// An operation started or finished (`FolderOpEvent`).
pub const FOLDER_OP: &str = "folder-op";
/// A `RemoteEvent` from the server follower.
pub const REMOTE: &str = "remote";
/// The browser sign-in ended (`LoginEvent`).
pub const LOGIN_FINISHED: &str = "login-finished";

/// What an operation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum OpAction {
    /// Scanning and comparing (changes nothing).
    Status,
    Sync,
    KeepLocal,
    KeepServer,
}

/// An operation began or ended.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum OpStage {
    Started,
    Finished {
        /// Present for sync and the conflict decisions when they succeeded.
        report: Option<Box<SyncReport>>,
        error: Option<CommandError>,
    },
}

/// `folder-op` payload.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct FolderOpEvent {
    pub folder: i64,
    pub action: OpAction,
    #[serde(flatten)]
    pub stage: OpStage,
}

/// `folder-status` payload: the status computed, or why it couldn't be.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct FolderStatusEvent {
    pub folder: i64,
    pub status: Option<FolderStatus>,
    pub error: Option<CommandError>,
}

/// `login-finished` payload.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum LoginEvent {
    Signed { account: Account },
    Failed { error: CommandError },
}

/// Emits an event; a failure (window closing) is only traced.
pub fn emit<P: Serialize + Clone>(app: &AppHandle, event: &str, payload: &P) {
    if let Err(error) = app.emit(event, payload) {
        tracing::debug!(%error, event, "could not emit an event");
    }
}
