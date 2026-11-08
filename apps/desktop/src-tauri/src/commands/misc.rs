//! Activity log, opening things in the OS, server version.

use cloudit_client::{ActivityEntry, OpContext};
use tauri::{AppHandle, State};

use super::{open_path, open_url};
use crate::error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub fn activity(
    state: State<'_, AppState>,
    folder: Option<i64>,
    limit: u32,
) -> Result<Vec<ActivityEntry>, CommandError> {
    Ok(state.engine.activity(folder, limit.min(1000))?)
}

#[tauri::command]
pub async fn server_version(state: State<'_, AppState>) -> Result<String, CommandError> {
    Ok(state.engine.server_version(&OpContext::new()).await?)
}

/// Opens a linked folder in the file manager. Only linked folders can be opened.
#[tauri::command]
pub fn open_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<(), CommandError> {
    let link = state.engine.folder(folder)?;
    open_path(&app, &link.local_path)
}

/// Opens the folder's history page of the web UI in the browser.
#[tauri::command]
pub fn open_history(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<(), CommandError> {
    let link = state.engine.folder(folder)?;
    let account = state
        .engine
        .account()?
        .ok_or(cloudit_client::Error::NotSignedIn)?;
    let url = format!(
        "{}/scopes/{}?folder={}",
        account.server_url.trim_end_matches('/'),
        link.scope_id,
        link.folder_id
    );
    open_url(&app, &url);
    Ok(())
}

/// Opens the log directory in the file manager.
#[tauri::command]
pub fn open_log_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    open_path(&app, &state.log_dir)
}
