//! The main window's life with the tray: it is created hidden (`tauri.conf.json`) and
//! shown at startup unless the app was launched by autostart ([`MINIMIZED_ARG`]);
//! closing it only hides it (the app keeps syncing in the tray; the first time a
//! notification says so); the tray, a second launch and "Open cloudit" show it again.

use tauri::{AppHandle, Manager, Window, WindowEvent};

use crate::i18n::{Msg, text};
use crate::notifications;
use crate::state::AppState;
use crate::tray;

/// The main window's label (`tauri.conf.json`).
pub const MAIN: &str = "main";
/// Passed by the autostart entry: start in the tray, without showing the window.
pub const MINIMIZED_ARG: &str = "--minimized";

/// Whether this process was started by autostart.
pub fn started_minimized() -> bool {
    std::env::args().skip(1).any(|arg| arg == MINIMIZED_ARG)
}

/// Shows, unminimizes and focuses the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The main window is visible and focused (the user sees what the UI shows).
pub fn in_front(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN).is_some_and(|w| {
        w.is_visible().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
            && !w.is_minimized().unwrap_or(false)
    })
}

/// Window events: closing the main window hides it instead (when the tray exists).
pub fn on_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    // Without a tray there would be no way back: closing quits as usual.
    if let WindowEvent::CloseRequested { api, .. } = event
        && tray::exists(window.app_handle())
    {
        api.prevent_close();
        if let Err(error) = window.hide() {
            tracing::warn!(%error, "could not hide the window");
        }
        first_close_notice(window.app_handle());
    }
}

/// Says once per device that the app is still running (whatever the notifications
/// switch: it explains where the window went).
fn first_close_notice(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state.engine.tray_notice_shown() {
        Ok(false) => {}
        Ok(true) => return,
        Err(error) => {
            tracing::warn!(%error, "could not read whether the tray notice was shown");
            return;
        }
    }
    if let Err(error) = state.engine.set_tray_notice_shown() {
        tracing::warn!(%error, "could not record the tray notice");
    }
    let lang = state.lang();
    notifications::send(
        app,
        &text(lang, Msg::TrayNoticeTitle),
        &text(lang, Msg::TrayNoticeBody),
    );
}
