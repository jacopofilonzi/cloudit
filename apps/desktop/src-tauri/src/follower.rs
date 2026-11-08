//! Follows the server's events while signed in and forwards them to the UI. A head or
//! conflict change also refreshes that folder's status (this milestone syncs by hand:
//! it only shows the news).

use cloudit_client::CancellationToken;
use cloudit_client::events::{self, RemoteEvent};
use tauri::{AppHandle, Manager};

use crate::events::{OpAction, REMOTE, emit};
use crate::ops;
use crate::state::{AppState, Connection};

/// (Re)starts the follower; call it after signing in and at startup when signed in.
pub fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    let cancel = CancellationToken::new();
    state.set_follower(Some(cancel.clone()));
    state.set_connection(Connection::Connecting);
    let engine = state.engine.clone();
    let app = app.clone();
    // `follow` spawns tasks, so it must run inside the runtime (not in `setup`).
    tauri::async_runtime::spawn(async move {
        let mut rx = events::follow(&engine, cancel);
        while let Some(event) = rx.recv().await {
            emit(&app, REMOTE, &event);
            match &event {
                RemoteEvent::Connected { .. } => {
                    app.state::<AppState>().set_connection(Connection::Online)
                }
                RemoteEvent::Disconnected { .. } => {
                    app.state::<AppState>().set_connection(Connection::Offline);
                }
                RemoteEvent::HeadChanged { folder, .. }
                | RemoteEvent::ConflictCreated { folder, .. } => {
                    refresh(&app, *folder);
                }
                RemoteEvent::SignedOut { error_kind } => {
                    tracing::warn!(?error_kind, "the server refused the token");
                    break;
                }
                RemoteEvent::FoldersChanged | RemoteEvent::UsageChanged => {}
            }
        }
        tracing::debug!("the server follower ended");
    });
}

/// Recomputes a folder's status in the background (the result goes out as events).
fn refresh(app: &AppHandle, folder: i64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<AppState>().engine.clone();
        let _ = ops::run(&app, folder, OpAction::Status, |ctx| async move {
            engine.status(folder, &ctx).await
        })
        .await;
    });
}
