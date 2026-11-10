//! Device-wide settings (automatic sync switch, global pause, notifications,
//! autostart), a folder's automatic sync settings, the running programs to pick from,
//! and what the scheduler shows of each folder.
//!
//! The settings are kept by the engine (`meta`) except autostart, which is the
//! autostart plugin's entry in the OS. Every change, from the UI, the tray or the end
//! of a pause, goes out as the `app-settings` event and rebuilds the tray, so both
//! stay consistent.

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use cloudit_client::{AutoSyncSettings, LinkedFolder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt as _};

use crate::auto::{AutoFolderState, Input, ProcessTable};
use crate::error::CommandError;
use crate::events::{APP_SETTINGS, emit};
use crate::state::AppState;

/// The tray's "Pause for 1 hour".
pub const TRAY_PAUSE_MINUTES: u32 = 60;
/// Longest pause, in minutes (a day).
const MAX_PAUSE_MINUTES: u32 = 24 * 60;

/// Device-wide settings of the app.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AppSettings {
    /// Automatic sync of the folders that have it on (default on).
    pub auto_sync: bool,
    /// Desktop notifications for conflicts and errors (default on).
    pub notifications: bool,
    /// The app starts with the system (in the tray).
    pub autostart: bool,
    /// Automatic sync is paused until then (RFC 3339 UTC; `None`: not paused).
    pub paused_until: Option<String>,
}

/// A change of the settings: omitted fields stay as they are.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AppSettingsUpdate {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub auto_sync: Option<bool>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub notifications: Option<bool>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub autostart: Option<bool>,
}

/// The current settings.
pub fn current(app: &AppHandle) -> Result<AppSettings, CommandError> {
    let engine = &app.state::<AppState>().engine;
    Ok(AppSettings {
        auto_sync: engine.auto_sync_enabled()?,
        notifications: engine.notifications_enabled()?,
        autostart: autostart_enabled(app),
        paused_until: engine
            .paused_until()?
            .map(|until| until.to_rfc3339_opts(SecondsFormat::Secs, true)),
    })
}

/// Whether the autostart entry exists (an unreadable one counts as off).
fn autostart_enabled(app: &AppHandle) -> bool {
    if app.try_state::<AutoLaunchManager>().is_none() {
        return false;
    }
    app.autolaunch().is_enabled().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the autostart entry");
        false
    })
}

fn set_autostart(app: &AppHandle, on: bool) -> Result<(), CommandError> {
    if app.try_state::<AutoLaunchManager>().is_none() {
        return Err(CommandError::internal("autostart is not available"));
    }
    let manager = app.autolaunch();
    let result = if on {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| CommandError::internal(format!("autostart: {e}")))
}

/// Tells the UI and the tray that the settings changed.
pub fn changed(app: &AppHandle) {
    match current(app) {
        Ok(settings) => emit(app, APP_SETTINGS, &settings),
        Err(error) => tracing::warn!(message = %error.message, "could not read the settings"),
    }
    app.state::<AppState>().tray.refresh();
}

/// Pauses automatic sync for `minutes` (1 to a day).
pub fn start_pause(app: &AppHandle, minutes: u32) -> Result<AppSettings, CommandError> {
    if !(1..=MAX_PAUSE_MINUTES).contains(&minutes) {
        return Err(CommandError::invalid_input(format!(
            "pause of {minutes} minutes"
        )));
    }
    let until: DateTime<Utc> = Utc::now() + TimeDelta::minutes(i64::from(minutes));
    let state = app.state::<AppState>();
    state.engine.set_paused_until(Some(until))?;
    state.auto.send(Input::Paused(Some(until)));
    tracing::info!(%until, "automatic sync paused");
    changed(app);
    current(app)
}

/// Ends the pause now.
pub fn end_pause(app: &AppHandle) -> Result<AppSettings, CommandError> {
    let state = app.state::<AppState>();
    state.engine.set_paused_until(None)?;
    state.auto.send(Input::Paused(None));
    tracing::info!("automatic sync resumed");
    changed(app);
    current(app)
}

#[tauri::command]
pub fn app_settings(app: AppHandle) -> Result<AppSettings, CommandError> {
    current(&app)
}

#[tauri::command]
pub fn set_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettingsUpdate,
) -> Result<AppSettings, CommandError> {
    if let Some(on) = settings.auto_sync {
        state.engine.set_auto_sync_enabled(on)?;
        state.auto.send(Input::Enabled(on));
    }
    if let Some(on) = settings.notifications {
        state.engine.set_notifications_enabled(on)?;
    }
    if let Some(on) = settings.autostart {
        set_autostart(&app, on)?;
    }
    changed(&app);
    current(&app)
}

#[tauri::command]
pub fn pause_for(app: AppHandle, minutes: u32) -> Result<AppSettings, CommandError> {
    start_pause(&app, minutes)
}

#[tauri::command]
pub fn resume(app: AppHandle) -> Result<AppSettings, CommandError> {
    end_pause(&app)
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
