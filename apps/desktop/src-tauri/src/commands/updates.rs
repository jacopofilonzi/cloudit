//! App updates (`updater`) and the versions shown in the settings: this app's, the
//! server's and the latest release's.

use cloudit_client::OpContext;
use serde::Serialize;
use tauri::{AppHandle, State};

use super::open_url;
use crate::error::CommandError;
use crate::state::AppState;
use crate::updater::{self, UpdatePhase, UpdateState};

/// The versions of the settings' "About" section.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct VersionInfo {
    pub app_version: String,
    /// `None`: not signed in, or the server couldn't be reached.
    pub server_version: Option<String>,
    /// The latest release the updater knows of (`None`: not checked yet).
    pub latest_version: Option<String>,
    /// The server runs a release older than the latest one (only its administrator
    /// can update it).
    pub server_outdated: bool,
}

/// The latest release known: the newer one found, or this app's after a check that
/// found nothing newer.
fn latest_known(state: &UpdateState) -> Option<String> {
    state
        .latest_version
        .clone()
        .or_else(|| (state.phase == UpdatePhase::UpToDate).then(|| state.current_version.clone()))
}

#[must_use]
fn server_outdated(server: Option<&str>, latest: Option<&str>) -> bool {
    match (server, latest) {
        (Some(server), Some(latest)) => cloudit_core::version::is_newer(latest, server),
        _ => false,
    }
}

#[tauri::command]
pub fn update_state(app: AppHandle) -> UpdateState {
    updater::current(&app)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> UpdateState {
    updater::check(&app).await
}

#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<UpdateState, CommandError> {
    updater::install(&app)
}

/// Opens the release page of the available version (or the latest release).
#[tauri::command]
pub fn open_release_page(app: AppHandle) {
    let url = updater::current(&app)
        .release_url
        .unwrap_or_else(|| "https://github.com/jacopofilonzi/cloudit/releases/latest".to_owned());
    open_url(&app, &url);
}

#[tauri::command]
pub async fn version_info(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<VersionInfo, CommandError> {
    let server_version = if state.engine.signed_in() {
        match state.engine.server_version(&OpContext::new()).await {
            Ok(version) => Some(version),
            Err(error) => {
                tracing::debug!(%error, "could not read the server version");
                None
            }
        }
    } else {
        None
    };
    let update = updater::current(&app);
    let latest_version = latest_known(&update);
    Ok(VersionInfo {
        app_version: update.current_version,
        server_outdated: server_outdated(server_version.as_deref(), latest_version.as_deref()),
        server_version,
        latest_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::Install;

    #[test]
    fn the_latest_known_release_and_outdated_servers() {
        let mut state = UpdateState::new("0.2.0".into(), Install::InApp);
        assert_eq!(latest_known(&state), None);
        state.checked(None, "t".into());
        assert_eq!(latest_known(&state).as_deref(), Some("0.2.0"));
        state.checked(Some(("0.3.0".into(), None)), "t".into());
        assert_eq!(latest_known(&state).as_deref(), Some("0.3.0"));

        assert!(server_outdated(Some("0.1.0"), Some("0.2.0")));
        assert!(!server_outdated(Some("0.2.0"), Some("0.2.0")));
        assert!(!server_outdated(Some("0.3.0"), Some("0.2.0")));
        assert!(!server_outdated(None, Some("0.2.0")));
        assert!(!server_outdated(Some("0.1.0"), None));
    }
}
