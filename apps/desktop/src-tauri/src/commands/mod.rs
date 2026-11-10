//! The commands the UI invokes (thin wrappers over `cloudit_client::Engine`). Every
//! command must be listed in `command_names.rs` and granted in the capability file.

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt as _;

use crate::error::CommandError;

pub mod account;
pub mod auto;
pub mod folders;
pub mod linking;
pub mod misc;
pub mod updates;

/// Set (to anything) to only log the URLs the app would open: automated runs drive the
/// browser themselves.
const NO_OPEN_ENV: &str = "CLOUDIT_DESKTOP_NO_OPEN";

/// Opens a URL in the default browser (a failure is logged: the UI can offer a retry).
pub(crate) fn open_url(app: &AppHandle, url: &str) {
    if std::env::var_os(NO_OPEN_ENV).is_some() {
        tracing::info!(%url, "not opening the browser (CLOUDIT_DESKTOP_NO_OPEN is set)");
        return;
    }
    if let Err(error) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!(%error, "could not open the browser");
    }
}

/// Opens a directory in the file manager.
pub(crate) fn open_path(app: &AppHandle, path: &Path) -> Result<(), CommandError> {
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| CommandError::internal(format!("opening {}: {e}", path.display())))
}
