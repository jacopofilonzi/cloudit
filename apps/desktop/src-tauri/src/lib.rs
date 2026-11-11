//! The cloudit desktop app (Tauri 2): a thin shell over `cloudit_client::Engine`.
//!
//! - `commands/`: the commands the UI invokes (account, folders, linking, misc);
//! - `ops`: runs one engine operation with cancellation, throttled progress and the
//!   started / finished / status events;
//! - `follower`: forwards the server's events while signed in;
//! - `auto`: the automatic sync scheduler (file watchers, associated programs, server
//!   events, retries) over `ops`;
//! - `updater`: in-app updates (checked by the backend, installed only when the user
//!   asks);
//! - `events`: event names and payloads (TypeScript twins in `src/lib/events.ts`);
//! - `error`: the serializable error (`kind` is the engine's stable `ErrorKind`).
//!
//! The UI never receives secrets: the device token stays in the engine (OS keychain).
//! Folder dialogs and "open in the file manager / browser" are done here, so the web
//! view needs no plugin permissions: its capability only allows the app's own commands
//! and listening to events.

mod auto;
mod commands;
mod compat;
mod error;
mod events;
mod follower;
mod i18n;
mod logging;
mod notifications;
mod ops;
mod state;
mod tray;
mod updater;
mod window;

use std::path::PathBuf;

use cloudit_client::{Engine, EngineConfig};
use tauri::{Manager, RunEvent};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};

use crate::state::AppState;

include!("command_names.rs");

/// Overrides the engine data directory (manual runs with a scratch state).
const DATA_DIR_ENV: &str = "CLOUDIT_DESKTOP_DATA_DIR";

/// Starts the app.
pub fn run() {
    // Only one instance may run: the single-instance plugin is registered FIRST, so a
    // second launch focuses the first window and exits before `setup` opens the engine
    // database, the token store or the server follower.
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg(window::MINIMIZED_ARG)
                .build(),
        )
        .on_window_event(window::on_event)
        .setup(|app| {
            let data_dir = match std::env::var_os(DATA_DIR_ENV) {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_local_data_dir()?,
            };
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            let guard = logging::init(&log_dir);
            tracing::info!(version = %app.package_info().version, data_dir = %data_dir.display(), "cloudit desktop starting");
            let opened = tauri::async_runtime::block_on(Engine::open(EngineConfig::new(&data_dir)));
            let engine = match opened {
                Ok(engine) => engine,
                Err(error) => {
                    tracing::error!(%error, "cannot open the engine");
                    app.dialog()
                        .message(format!(
                            "cloudit cannot start: its local data could not be opened.\n\n{error}"
                        ))
                        .kind(MessageDialogKind::Error)
                        .title("cloudit")
                        .blocking_show();
                    return Err(error.to_string().into());
                }
            };
            let signed_in = engine.signed_in();
            let (auto_handle, auto_inputs) = auto::channel();
            let (tray_handle, tray_inputs) = tray::channel();
            let alerts = notifications::Alerts::new(
                engine
                    .folders()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| (f.id, f.last_state)),
            );
            app.manage(AppState::new(
                engine,
                data_dir,
                log_dir,
                guard,
                auto_handle,
                tray_handle,
                alerts,
            ));
            // Without a tray (e.g. no AppIndicator on Linux) closing the window quits.
            if let Err(error) = tray::start(app.handle(), tray_inputs) {
                tracing::warn!(%error, "no tray icon");
            }
            if !window::started_minimized() || !tray::exists(app.handle()) {
                window::show_main(app.handle());
            }
            updater::start(app.handle());
            auto::start(app.handle(), auto_inputs);
            if signed_in {
                follower::start(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::account::app_info,
            commands::account::session,
            commands::account::suggest_device_name,
            commands::account::normalize_server_url,
            commands::account::start_login,
            commands::account::reopen_login_browser,
            commands::account::cancel_login,
            commands::account::sign_out,
            commands::account::check_compat,
            commands::folders::folders,
            commands::folders::refresh_status,
            commands::folders::refresh_all,
            commands::folders::sync_folder,
            commands::folders::sync_all,
            commands::folders::keep_local,
            commands::folders::keep_server,
            commands::folders::decide_later,
            commands::folders::cancel_op,
            commands::linking::scopes,
            commands::linking::server_folders,
            commands::linking::create_scope,
            commands::linking::pick_folder,
            commands::linking::link_folder,
            commands::linking::create_and_link,
            commands::linking::unlink,
            commands::linking::relocate,
            commands::linking::set_ignore,
            commands::linking::set_paused,
            commands::linking::refresh_names,
            commands::misc::activity,
            commands::misc::open_folder,
            commands::misc::open_history,
            commands::misc::open_log_dir,
            commands::misc::server_version,
            commands::misc::set_ui_language,
            commands::auto::app_settings,
            commands::auto::set_app_settings,
            commands::auto::pause_for,
            commands::auto::resume,
            commands::auto::set_auto_sync,
            commands::auto::running_processes,
            commands::auto::auto_states,
            commands::updates::update_state,
            commands::updates::check_update,
            commands::updates::install_update,
            commands::updates::open_release_page,
            commands::updates::version_info,
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            tracing::error!(%error, "cannot start the app");
            eprintln!("cloudit: cannot start: {error}");
            return;
        }
    };
    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(state) = handle.try_state::<AppState>()
        {
            state.cancel_all();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::COMMANDS;

    /// The capability grants exactly the app's commands (plus event listening).
    #[test]
    fn capability_matches_the_command_list() {
        let text = include_str!("../capabilities/default.json");
        let json: serde_json::Value = serde_json::from_str(text).expect("capability JSON");
        let granted: std::collections::BTreeSet<String> = json["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .filter_map(|p| p.as_str())
            .filter_map(|p| p.strip_prefix("allow-"))
            .map(|p| p.replace('-', "_"))
            .collect();
        let wanted: std::collections::BTreeSet<String> =
            COMMANDS.iter().map(|c| (*c).to_owned()).collect();
        assert_eq!(granted, wanted);
    }
}
