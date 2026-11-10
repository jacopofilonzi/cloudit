//! Linked folders: listing, status, sync and the conflict decisions.

use std::sync::Arc;

use cloudit_client::{FolderStatus, LinkedFolder, OpContext, SyncReport};
use futures::StreamExt as _;
use tauri::{AppHandle, State};
use tokio::sync::Semaphore;

use crate::auto::Input;
use crate::error::CommandError;
use crate::events::OpAction;
use crate::ops;
use crate::state::AppState;

/// Folders worked on at the same time by "check all" / "sync all".
const PARALLEL: usize = 3;

#[tauri::command]
pub fn folders(state: State<'_, AppState>) -> Result<Vec<LinkedFolder>, CommandError> {
    Ok(state.engine.folders()?)
}

/// Computes one folder's status (scan + server head); also reported as events.
#[tauri::command]
pub async fn refresh_status(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<FolderStatus, CommandError> {
    let engine = state.engine.clone();
    ops::run(&app, folder, OpAction::Status, |ctx| async move {
        engine.status(folder, &ctx).await
    })
    .await
}

/// Computes every folder's status in the background (events carry the results).
#[tauri::command]
pub fn refresh_all(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    let ids: Vec<i64> = state.engine.folders()?.iter().map(|f| f.id).collect();
    spawn_all(app, ids, OpAction::Status);
    Ok(())
}

/// Syncs one folder (refused while paused).
#[tauri::command]
pub async fn sync_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<SyncReport, CommandError> {
    let engine = state.engine.clone();
    ops::run(&app, folder, OpAction::Sync, |ctx| async move {
        engine.sync(folder, &ctx).await
    })
    .await
}

/// Syncs every folder that isn't paused, a few at a time, in the background.
#[tauri::command]
pub fn sync_all(app: AppHandle) -> Result<(), CommandError> {
    sync_all_folders(&app)
}

/// Starts a manual sync of every folder that isn't paused (also the tray's "Sync all").
pub(crate) fn sync_all_folders(app: &AppHandle) -> Result<(), CommandError> {
    let ids: Vec<i64> = tauri::Manager::state::<AppState>(app)
        .engine
        .folders()?
        .iter()
        .filter(|f| !f.paused)
        .map(|f| f.id)
        .collect();
    spawn_all(app.clone(), ids, OpAction::Sync);
    Ok(())
}

/// Runs `action` (status or a manual sync) on `ids`, a few at a time, in the background.
pub(crate) fn spawn_all(app: AppHandle, ids: Vec<i64>, action: OpAction) {
    tauri::async_runtime::spawn(async move {
        let limit = Arc::new(Semaphore::new(PARALLEL));
        futures::stream::iter(ids)
            .for_each_concurrent(None, |folder| {
                let app = app.clone();
                let limit = limit.clone();
                async move {
                    let Ok(_permit) = limit.acquire().await else {
                        return;
                    };
                    let engine = tauri::Manager::state::<AppState>(&app).engine.clone();
                    // Results reach the UI as events.
                    let _ = match action {
                        OpAction::Status => {
                            ops::run(&app, folder, action, |ctx: OpContext| async move {
                                engine.status(folder, &ctx).await
                            })
                            .await
                            .map(drop)
                        }
                        _ => ops::run(&app, folder, action, |ctx: OpContext| async move {
                            engine.sync(folder, &ctx).await
                        })
                        .await
                        .map(drop),
                    };
                }
            })
            .await;
    });
}

/// "Keep this PC's version" (the conflict snapshot this device uploaded).
#[tauri::command]
pub async fn keep_local(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<SyncReport, CommandError> {
    let engine = state.engine.clone();
    ops::run(&app, folder, OpAction::KeepLocal, |ctx| async move {
        engine.keep_local(folder, None, &ctx).await
    })
    .await
}

/// "Keep the server's version".
#[tauri::command]
pub async fn keep_server(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: i64,
) -> Result<SyncReport, CommandError> {
    let engine = state.engine.clone();
    ops::run(&app, folder, OpAction::KeepServer, |ctx| async move {
        engine.keep_server(folder, None, &ctx).await
    })
    .await
}

/// "Decide later": pauses the folder.
#[tauri::command]
pub fn decide_later(state: State<'_, AppState>, folder: i64) -> Result<LinkedFolder, CommandError> {
    let link = state.engine.decide_later(folder)?;
    state.auto.send(Input::FoldersChanged);
    Ok(link)
}

/// Cancels what is running on a folder; whether anything was.
#[tauri::command]
pub fn cancel_op(state: State<'_, AppState>, folder: i64) -> bool {
    state.cancel_folder(folder)
}
