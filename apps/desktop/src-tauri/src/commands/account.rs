//! Account commands: app info, session, browser sign-in, sign-out.

use cloudit_client::{Account, CancellationToken, OpContext, platform};
use serde::Serialize;
use tauri::{AppHandle, State};

use super::open_url;
use crate::error::CommandError;
use crate::events::{LOGIN_FINISHED, LoginEvent, emit};
use crate::follower;
use crate::state::{AppState, Connection};

/// Static facts about this installation.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub data_dir: String,
    pub log_dir: String,
}

/// Who is signed in on this device.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct SessionState {
    pub account: Option<Account>,
    /// A usable token is loaded (false after a revoked / lost token: sign in again).
    pub signed_in: bool,
    /// The token is in the fallback file, not the OS keychain.
    pub token_warning: bool,
    /// The state of the server stream at this moment.
    pub connection: Connection,
}

/// A sign-in that started: the page the browser was sent to.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct LoginStarted {
    pub browser_url: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        os: platform::os_name().to_owned(),
        data_dir: state.data_dir.display().to_string(),
        log_dir: state.log_dir.display().to_string(),
    }
}

#[tauri::command]
pub fn session(state: State<'_, AppState>) -> Result<SessionState, CommandError> {
    let account = state.engine.account()?;
    Ok(SessionState {
        token_warning: account.as_ref().is_some_and(Account::token_warning),
        signed_in: account.is_some() && state.engine.signed_in(),
        connection: state.connection(),
        account,
    })
}

#[tauri::command]
pub fn suggest_device_name() -> String {
    platform::device_name()
}

/// Turns what the user typed into the server URL to use: `host` or `host:port` get
/// `https://` (`http://` for local addresses), trailing slashes go.
#[tauri::command]
pub fn normalize_server_url(raw: String) -> Result<String, CommandError> {
    normalize(&raw)
}

fn normalize(raw: &str) -> Result<String, CommandError> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        let host = raw.split(['/', ':']).next().unwrap_or_default();
        let local = host == "localhost"
            || host.starts_with("127.")
            || host.starts_with("192.168.")
            || host.starts_with("10.")
            || host.ends_with(".local")
            || host.ends_with(".lan");
        format!("{}://{raw}", if local { "http" } else { "https" })
    };
    let url = cloudit_client::api::parse_server_url(&with_scheme)?;
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// Checks the server, opens the browser and waits for the approval in the background;
/// the outcome arrives as a `login-finished` event.
#[tauri::command]
pub async fn start_login(
    app: AppHandle,
    state: State<'_, AppState>,
    server_url: String,
    device_name: Option<String>,
) -> Result<LoginStarted, CommandError> {
    let server_url = normalize(&server_url)?;
    state.cancel_login();
    let cancel = CancellationToken::new();
    let ctx = OpContext::new().with_cancel(cancel.clone());
    let pending = state
        .engine
        .begin_login(&server_url, device_name.as_deref(), &ctx)
        .await?;
    let browser_url = pending.browser_url().to_owned();
    let id = state.set_login(cancel, browser_url.clone());
    tracing::info!(url = %browser_url, "opening the browser to sign in");
    open_url(&app, &browser_url);

    let engine = state.engine.clone();
    tauri::async_runtime::spawn({
        let app = app.clone();
        async move {
            let result = engine.finish_login(pending, &ctx).await;
            let state = tauri::Manager::state::<AppState>(&app);
            state.end_login(id);
            match result {
                Ok(account) => {
                    tracing::info!(device = %account.device_name, "signed in");
                    follower::start(&app);
                    emit(&app, LOGIN_FINISHED, &LoginEvent::Signed { account });
                }
                Err(error) => {
                    tracing::info!(%error, "sign-in did not complete");
                    emit(
                        &app,
                        LOGIN_FINISHED,
                        &LoginEvent::Failed {
                            error: error.into(),
                        },
                    );
                }
            }
        }
    });
    Ok(LoginStarted { browser_url })
}

#[tauri::command]
pub fn reopen_login_browser(app: AppHandle, state: State<'_, AppState>) -> bool {
    match state.login_url() {
        Some(url) => {
            open_url(&app, &url);
            true
        }
        None => false,
    }
}

#[tauri::command]
pub fn cancel_login(state: State<'_, AppState>) -> bool {
    state.cancel_login()
}

/// Forgets the token and the account on this device; folder links stay for the next
/// sign-in to the same account.
#[tauri::command]
pub async fn sign_out(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.cancel_all();
    state.engine.logout().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_completed_and_trimmed() {
        let ok = |raw: &str| normalize(raw).ok();
        assert_eq!(
            ok("sync.example.org").as_deref(),
            Some("https://sync.example.org")
        );
        assert_eq!(
            ok(" https://sync.example.org/ ").as_deref(),
            Some("https://sync.example.org")
        );
        assert_eq!(
            ok("localhost:8080").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(
            ok("192.168.1.5:8080/cloudit/").as_deref(),
            Some("http://192.168.1.5:8080/cloudit")
        );
        assert_eq!(ok("sync.casa.lan").as_deref(), Some("http://sync.casa.lan"));
        assert!(ok("ftp://example.org").is_none());
        assert!(ok("").is_none());
    }
}
