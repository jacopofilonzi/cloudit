//! Whether the signed-in server works with this app ([`cloudit_client::compat`]), as
//! the desktop app tracks it in `AppState::compat`.
//!
//! Learned from the follower (`RemoteEvent::Compatibility`, checked before every
//! reconnect), from every operation (an `app_outdated` / `server_outdated` error says
//! it doesn't, any success that it does), from the `check_compat` command and from a
//! sign-in (the engine refuses an incompatible server there). A change goes to the UI
//! (`compat` event), to the scheduler (no automatic run while incompatible) and to the
//! tray, and becoming incompatible is notified once.

use cloudit_client::Compat;
use tauri::{AppHandle, Manager};

use crate::auto::Input;
use crate::events::{COMPAT, emit};
use crate::notifications;
use crate::state::AppState;

/// Records what was learned and tells everyone when it changed.
pub fn update(app: &AppHandle, compat: Compat) {
    let state = app.state::<AppState>();
    if !state.set_compat(compat.clone()) {
        return;
    }
    let ok = compat.is_ok();
    if ok {
        tracing::info!("the server works with this app again");
    } else {
        tracing::warn!(
            ?compat,
            "the server doesn't work with this app: sync stopped"
        );
    }
    emit(app, COMPAT, &compat);
    state.auto.send(Input::Compatible(ok));
    state.tray.refresh();
    if let Some(notice) = state.alerts(|a| a.compat(ok)) {
        notifications::show(app, notice);
    }
}
