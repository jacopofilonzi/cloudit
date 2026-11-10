//! The app state (managed by Tauri): the engine, the automatic sync scheduler handle and
//! the registries of what is running, so the UI can cancel it and sign-out can stop
//! everything.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use cloudit_client::{CancellationToken, Engine};
use tracing_appender::non_blocking::WorkerGuard;

use crate::auto::{self, AutoFolderState};
use crate::i18n::Lang;
use crate::notifications::Alerts;
use crate::tray;

/// Whether the stream of server events is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum Connection {
    Connecting = 0,
    Online = 1,
    Offline = 2,
}

/// A browser sign-in waiting for the user.
struct LoginSession {
    id: u64,
    cancel: CancellationToken,
    browser_url: String,
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    /// Running operations by linked folder (a second one waits for the engine lock).
    ops: HashMap<i64, Vec<(u64, CancellationToken)>>,
    /// When the last operation on each folder ended (its file events are its own).
    ended: HashMap<i64, Instant>,
    login: Option<LoginSession>,
    follower: Option<CancellationToken>,
}

/// Shared state.
pub struct AppState {
    pub engine: Engine,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    /// The automatic sync scheduler.
    pub auto: auto::Handle,
    /// Its last view for the UI (`auto_states`).
    auto_view: Mutex<Vec<AutoFolderState>>,
    /// Rebuilds the tray menu.
    pub tray: tray::Handle,
    /// The UI language, for the texts the backend shows (tray, notifications).
    lang: Mutex<Lang>,
    /// What was already notified.
    alerts: Mutex<Alerts>,
    registry: Mutex<Registry>,
    /// The server stream: `Connection` as a number (the UI may load after the event).
    connection: AtomicU8,
    /// Keeps the log writer thread alive (flushes on drop).
    _log_guard: Option<WorkerGuard>,
}

impl AppState {
    pub fn new(
        engine: Engine,
        data_dir: PathBuf,
        log_dir: PathBuf,
        log_guard: Option<WorkerGuard>,
        auto: auto::Handle,
        tray: tray::Handle,
        alerts: Alerts,
    ) -> Self {
        Self {
            engine,
            data_dir,
            log_dir,
            auto,
            auto_view: Mutex::new(Vec::new()),
            tray,
            lang: Mutex::new(Lang::default()),
            alerts: Mutex::new(alerts),
            registry: Mutex::new(Registry::default()),
            connection: AtomicU8::new(0),
            _log_guard: log_guard,
        }
    }

    /// The UI language.
    pub fn lang(&self) -> Lang {
        *self
            .lang
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records the UI language; whether it changed.
    pub fn set_lang(&self, lang: Lang) -> bool {
        let mut current = self
            .lang
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *current, lang) != lang
    }

    /// Runs `f` on the notification dedupe state.
    pub fn alerts<T>(&self, f: impl FnOnce(&mut Alerts) -> T) -> T {
        f(&mut self
            .alerts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// Records whether the server stream is up.
    pub fn set_connection(&self, connection: Connection) {
        self.connection.store(connection as u8, Ordering::Relaxed);
    }

    pub fn connection(&self) -> Connection {
        match self.connection.load(Ordering::Relaxed) {
            1 => Connection::Online,
            2 => Connection::Offline,
            _ => Connection::Connecting,
        }
    }

    /// Locks the registry (a poisoned lock still holds consistent data: only inserts
    /// and removals happen under it).
    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Registers a running operation; the guard unregisters it.
    pub fn begin_op(&self, folder: i64) -> OpGuard<'_> {
        let token = CancellationToken::new();
        let mut registry = self.registry();
        registry.next_id += 1;
        let id = registry.next_id;
        registry
            .ops
            .entry(folder)
            .or_default()
            .push((id, token.clone()));
        OpGuard {
            state: self,
            folder,
            id,
            token,
        }
    }

    /// Whether an operation runs on `folder` or ended less than `grace` ago.
    pub fn folder_busy(&self, folder: i64, grace: Duration) -> bool {
        let registry = self.registry();
        registry.ops.contains_key(&folder)
            || registry
                .ended
                .get(&folder)
                .is_some_and(|at| at.elapsed() < grace)
    }

    /// Whether any operation runs on any folder (an update waits for them).
    pub fn any_op_running(&self) -> bool {
        !self.registry().ops.is_empty()
    }

    /// Records the scheduler's view for the UI.
    pub fn set_auto_view(&self, view: Vec<AutoFolderState>) {
        *self
            .auto_view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = view;
    }

    /// What automatic sync shows of each automatic folder.
    pub fn auto_view(&self) -> Vec<AutoFolderState> {
        self.auto_view
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether automatic sync currently handles `folder` (it syncs on server events).
    pub fn is_auto(&self, folder: i64) -> bool {
        self.auto_view().iter().any(|f| f.folder == folder)
    }

    /// Cancels every running operation of a folder; whether any was running.
    pub fn cancel_folder(&self, folder: i64) -> bool {
        let registry = self.registry();
        let Some(ops) = registry.ops.get(&folder) else {
            return false;
        };
        for (_, token) in ops {
            token.cancel();
        }
        !ops.is_empty()
    }

    /// Cancels everything running (sign-out, exit).
    pub fn cancel_all(&self) {
        let mut registry = self.registry();
        for ops in registry.ops.values() {
            for (_, token) in ops {
                token.cancel();
            }
        }
        if let Some(login) = registry.login.take() {
            login.cancel.cancel();
        }
        if let Some(follower) = registry.follower.take() {
            follower.cancel();
        }
    }

    /// Starts tracking a sign-in (cancelling a previous one); returns its id.
    pub fn set_login(&self, cancel: CancellationToken, browser_url: String) -> u64 {
        let mut registry = self.registry();
        registry.next_id += 1;
        let id = registry.next_id;
        if let Some(old) = registry.login.replace(LoginSession {
            id,
            cancel,
            browser_url,
        }) {
            old.cancel.cancel();
        }
        id
    }

    /// Forgets the sign-in `id` (when it ended on its own).
    pub fn end_login(&self, id: u64) {
        let mut registry = self.registry();
        if registry.login.as_ref().is_some_and(|l| l.id == id) {
            registry.login = None;
        }
    }

    /// Cancels the sign-in in progress; whether there was one.
    pub fn cancel_login(&self) -> bool {
        match self.registry().login.take() {
            Some(login) => {
                login.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// The URL of the sign-in in progress.
    pub fn login_url(&self) -> Option<String> {
        self.registry()
            .login
            .as_ref()
            .map(|l| l.browser_url.clone())
    }

    /// Replaces the server follower, cancelling the previous one.
    pub fn set_follower(&self, token: Option<CancellationToken>) {
        let mut registry = self.registry();
        if let Some(old) = std::mem::replace(&mut registry.follower, token) {
            old.cancel();
        }
    }
}

/// A registered operation.
pub struct OpGuard<'a> {
    state: &'a AppState,
    folder: i64,
    id: u64,
    pub token: CancellationToken,
}

impl Drop for OpGuard<'_> {
    fn drop(&mut self) {
        let mut registry = self.state.registry();
        if let Some(ops) = registry.ops.get_mut(&self.folder) {
            ops.retain(|(id, _)| *id != self.id);
            if ops.is_empty() {
                registry.ops.remove(&self.folder);
            }
        }
        registry.ended.insert(self.folder, Instant::now());
    }
}
