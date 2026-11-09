//! Automatic sync: the device-wide switch, a folder's settings, the running programs
//! to pick from, and what the scheduler shows of each folder.

use cloudit_client::{AutoSyncSettings, LinkedFolder};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::auto::{AutoFolderState, Input, ProcessTable};
use crate::error::CommandError;
use crate::state::AppState;

/// Device-wide settings of the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AppSettings {
    /// Automatic sync of the folders that have it on (default on).
    pub auto_sync: bool,
}

#[tauri::command]
pub fn app_settings(state: State<'_, AppState>) -> Result<AppSettings, CommandError> {
    Ok(AppSettings {
        auto_sync: state.engine.auto_sync_enabled()?,
    })
}

#[tauri::command]
pub fn set_app_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    state.engine.set_auto_sync_enabled(settings.auto_sync)?;
    state.auto.send(Input::Enabled(settings.auto_sync));
    Ok(settings)
}

#[tauri::command]
pub fn set_auto_sync(
    state: State<'_, AppState>,
    folder: i64,
    settings: AutoSyncSettings,
) -> Result<LinkedFolder, CommandError> {
    let link = state.engine.set_auto_sync(folder, &settings)?;
    state.auto.send(Input::FoldersChanged);
    Ok(link)
}

/// The executable names of the running programs (sorted, no duplicates).
#[tauri::command]
pub async fn running_processes() -> Result<Vec<String>, CommandError> {
    tauri::async_runtime::spawn_blocking(|| ProcessTable::new().map(|mut table| table.list()))
        .await
        .map_err(|e| CommandError::internal(format!("process task: {e}")))?
        .map_err(CommandError::internal)
}

#[tauri::command]
pub fn auto_states(state: State<'_, AppState>) -> Vec<AutoFolderState> {
    state.auto_view()
}
