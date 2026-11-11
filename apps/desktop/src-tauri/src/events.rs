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
/// What automatic sync shows of each automatic folder (`AutoFolderState[]`).
pub const AUTO_STATE: &str = "auto-state";
/// The browser sign-in ended (`LoginEvent`).
pub const LOGIN_FINISHED: &str = "login-finished";
/// The device-wide settings changed, e.g. paused from the tray (`AppSettings`).
pub const APP_SETTINGS: &str = "app-settings";
/// The updater's state changed (`UpdateState`).
pub const UPDATE_STATE: &str = "update-state";
/// The tray asks the UI to show a folder's page (`NavigateEvent`).
pub const NAVIGATE: &str = "navigate";
/// Whether the server works with this app changed (`Compat`).
pub const COMPAT: &str = "compat";

/// Which page of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum NavigatePage {
    /// The folder in the list, with its detail.
    Detail,
    /// The conflict decision page.
    Conflict,
}

/// `navigate` payload.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct NavigateEvent {
    pub folder: i64,
    pub page: NavigatePage,
}

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
    /// Started by automatic sync (not by the user).
    pub auto: bool,
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
