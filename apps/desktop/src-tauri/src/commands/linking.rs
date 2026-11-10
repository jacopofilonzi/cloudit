//! Server listings and linking local folders to server folders.

use std::path::PathBuf;

use cloudit_client::{LinkedFolder, OpContext};
use cloudit_core::protocol::{FolderInfo, ScopeInfo};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt as _;

use crate::auto::Input;
use crate::error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub async fn scopes(state: State<'_, AppState>) -> Result<Vec<ScopeInfo>, CommandError> {
    Ok(state.engine.scopes(&OpContext::new()).await?)
}

#[tauri::command]
pub async fn server_folders(
    state: State<'_, AppState>,
    scope_id: String,
) -> Result<Vec<FolderInfo>, CommandError> {
    Ok(state
        .engine
        .server_folders(&scope_id, &OpContext::new())
        .await?)
}

#[tauri::command]
pub async fn create_scope(
    state: State<'_, AppState>,
    name: String,
) -> Result<ScopeInfo, CommandError> {
    Ok(state.engine.create_scope(&name, &OpContext::new()).await?)
}

/// Asks the user for a folder (native dialog); `None` when cancelled.
#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    title: String,
    start: Option<String>,
) -> Result<Option<String>, CommandError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title(title);
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        if let Some(start) = start.filter(|s| !s.is_empty()) {
            dialog = dialog.set_directory(PathBuf::from(start));
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| CommandError::internal(format!("folder dialog: {e}")))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| CommandError::internal(format!("folder dialog: {e}")))?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub async fn link_folder(
    state: State<'_, AppState>,
    folder_id: String,
    local_path: String,
    ignore: Vec<String>,
) -> Result<LinkedFolder, CommandError> {
    let result = state
        .engine
        .link_folder(
            &folder_id,
            &PathBuf::from(local_path),
            &ignore,
            &OpContext::new(),
        )
        .await;
    changed(&state, result)
}

#[tauri::command]
pub async fn create_and_link(
    state: State<'_, AppState>,
    scope_id: String,
    name: String,
    emoji: Option<String>,
    local_path: String,
    ignore: Vec<String>,
) -> Result<LinkedFolder, CommandError> {
    let result = state
        .engine
        .create_and_link(
            &scope_id,
            &name,
            emoji.as_deref().filter(|e| !e.is_empty()),
            &PathBuf::from(local_path),
            &ignore,
            &OpContext::new(),
        )
        .await;
    changed(&state, result)
}

#[tauri::command]
pub async fn unlink(state: State<'_, AppState>, folder: i64) -> Result<(), CommandError> {
    state.cancel_folder(folder);
    let result = state.engine.unlink(folder).await;
    changed(&state, result)
}

#[tauri::command]
pub async fn relocate(
    state: State<'_, AppState>,
    folder: i64,
    local_path: String,
) -> Result<LinkedFolder, CommandError> {
    let result = state
        .engine
        .relocate(folder, &PathBuf::from(local_path))
        .await;
    changed(&state, result)
}

#[tauri::command]
pub async fn set_ignore(
    state: State<'_, AppState>,
    folder: i64,
    patterns: Vec<String>,
) -> Result<LinkedFolder, CommandError> {
    let result = state.engine.set_ignore(folder, &patterns).await;
    changed(&state, result)
}

#[tauri::command]
pub fn set_paused(
    state: State<'_, AppState>,
    folder: i64,
    paused: bool,
) -> Result<LinkedFolder, CommandError> {
    changed(&state, state.engine.set_paused(folder, paused))
}

/// Refreshes the cached scope / folder names from the server.
#[tauri::command]
pub async fn refresh_names(state: State<'_, AppState>) -> Result<Vec<LinkedFolder>, CommandError> {
    let folders = state.engine.refresh_names(&OpContext::new()).await?;
    state.tray.refresh();
    Ok(folders)
}

/// A command that changed the folder list or a folder's settings: tells the automatic
/// sync scheduler and the tray when it succeeded.
fn changed<T>(state: &AppState, result: cloudit_client::Result<T>) -> Result<T, CommandError> {
    let value = result?;
    state.auto.send(Input::FoldersChanged);
    state.tray.refresh();
    Ok(value)
}
