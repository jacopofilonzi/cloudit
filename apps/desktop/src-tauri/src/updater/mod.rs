//! The in-app updater (`tauri-plugin-updater`), driven by the backend only: the web
//! view has no updater permission, it uses the app's `update_state`, `check_update`
//! and `install_update` commands and the `update-state` event.
//!
//! - **Checks**: [`FIRST_CHECK`] after startup, then every [`CHECK_EVERY`], and on
//!   request. The endpoint (`tauri.conf.json`: the latest GitHub release's
//!   `latest.json`) lists a signed package per platform; the plugin answers only when
//!   that version is newer than the running one. A check never downloads anything.
//! - **Never without the user**: an available version is shown (UI banner with the
//!   notes, tray item "Update to X.Y.Z"); only the user's **Update** starts
//!   [`install`]. It holds automatic sync (`auto::Input::Updating`), waits until no
//!   operation runs on any folder, downloads (progress in `update-state`), verifies
//!   the signature against the public key in `tauri.conf.json`, installs and restarts
//!   (Windows: the NSIS installer in quiet mode replaces the app and starts it again;
//!   AppImage: the file is replaced, then the app restarts). A failure releases
//!   automatic sync and can be retried.
//! - **How a build updates** ([`Install`]): Windows and AppImage in the app; a `.deb`
//!   install (Linux without `$APPIMAGE`) only links to the release page; development
//!   builds (`debug_assertions`) only show what is available.
//!
//! The pure state machine is `model.rs`.

mod model;

pub use model::{Install, UpdatePhase, UpdateState};

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt as _};

use crate::auto::Input;
use crate::error::CommandError;
use crate::events::{UPDATE_STATE, emit};
use crate::state::AppState;

/// The first check, after startup (the app settles first).
pub const FIRST_CHECK: Duration = Duration::from_secs(5);
/// Time between two automatic checks.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How often an install looks again whether the running operations ended.
const WAIT_POLL: Duration = Duration::from_millis(500);
/// Download progress goes out at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// The updater's state, managed by Tauri.
pub struct Updater {
    state: Mutex<UpdateState>,
    /// The update the last check found (what [`install`] installs).
    update: Mutex<Option<Update>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Manages the updater and starts the periodic checks (in `setup`).
pub fn start(app: &AppHandle) {
    let install = Install::detect(
        cfg!(debug_assertions),
        cfg!(windows),
        // Set by the AppImage runtime (what `tauri::Env::appimage` reads on Linux).
        std::env::var_os("APPIMAGE").is_some(),
    );
    let version = app.package_info().version.to_string();
    tracing::info!(?install, "updater ready");
    app.manage(Updater {
        state: Mutex::new(UpdateState::new(version, install)),
        update: Mutex::new(None),
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(&app).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The current state (a default one if the updater isn't managed).
pub fn current(app: &AppHandle) -> UpdateState {
    match app.try_state::<Updater>() {
        Some(updater) => lock(&updater.state).clone(),
        None => UpdateState::new(app.package_info().version.to_string(), Install::Never),
    }
}

/// Changes the state and tells the UI and the tray.
fn change(app: &AppHandle, f: impl FnOnce(&mut UpdateState)) -> UpdateState {
    let Some(updater) = app.try_state::<Updater>() else {
        return current(app);
    };
    let state = {
        let mut state = lock(&updater.state);
        f(&mut state);
        state.clone()
    };
    emit(app, UPDATE_STATE, &state);
    app.state::<AppState>().tray.refresh();
    state
}

/// Checks for a newer version now (skipped while checking or installing).
pub async fn check(app: &AppHandle) -> UpdateState {
    let Some(updater) = app.try_state::<Updater>() else {
        return current(app);
    };
    {
        let mut state = lock(&updater.state);
        if !state.may_check() {
            return state.clone();
        }
        state.checking();
    }
    change(app, |_| {});
    let found = match app.updater() {
        Ok(plugin) => plugin.check().await,
        Err(error) => Err(error),
    };
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    match found {
        Ok(Some(update)) => {
            tracing::info!(version = %update.version, "an app update is available");
            let found = (update.version.clone(), update.body.clone());
            *lock(&updater.update) = Some(update);
            change(app, |s| s.checked(Some(found), now))
        }
        Ok(None) => {
            *lock(&updater.update) = None;
            change(app, |s| s.checked(None, now))
        }
        Err(error) => {
            tracing::warn!(%error, "could not check for app updates");
            change(app, |s| s.check_failed(error.to_string()))
        }
    }
}

/// Starts installing the available update (the user asked): see the module docs.
pub fn install(app: &AppHandle) -> Result<UpdateState, CommandError> {
    let updater = app
        .try_state::<Updater>()
        .ok_or_else(|| CommandError::internal("the updater is not available"))?;
    let update = {
        let mut state = lock(&updater.state);
        if !state.may_install() {
            return Err(CommandError::invalid_input(format!(
                "no update to install now ({:?})",
                state.phase
            )));
        }
        let update = lock(&updater.update)
            .clone()
            .ok_or_else(|| CommandError::internal("the update is no longer known"))?;
        state.waiting();
        update
    };
    let started = change(app, |_| {});
    let appstate = app.state::<AppState>();
    appstate.auto.send(Input::Updating(true));
    tracing::info!(version = %update.version, "installing the app update");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_install(&app, &update).await {
            tracing::warn!(%error, "the app update failed");
            app.state::<AppState>().auto.send(Input::Updating(false));
            change(&app, |s| s.failed(error));
        }
    });
    Ok(started)
}

/// Waits for the running operations, downloads, verifies, installs and restarts.
async fn run_install(app: &AppHandle, update: &Update) -> Result<(), String> {
    while app.state::<AppState>().any_op_running() {
        tokio::time::sleep(WAIT_POLL).await;
    }
    let mut last: Option<Instant> = None;
    let bytes = update
        .download(
            |chunk, total| {
                let chunk = u64::try_from(chunk).unwrap_or(u64::MAX);
                let Some(updater) = app.try_state::<Updater>() else {
                    return;
                };
                lock(&updater.state).progress(chunk, total);
                if last.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                    last = Some(Instant::now());
                    change(app, |_| {});
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("download: {e}"))?;
    change(app, UpdateState::installing);
    // Verified again by `install` (signature, then the package format).
    update.install(bytes).map_err(|e| format!("install: {e}"))?;
    // Windows never gets here: the installer took over and the app exited.
    tracing::info!("app update installed: restarting");
    app.restart();
}
