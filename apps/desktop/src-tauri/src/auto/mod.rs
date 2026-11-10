//! Automatic sync: one scheduler task for the app's lifetime. It decides when each
//! automatic folder syncs (the pure rules are in `plan.rs`) and runs it through
//! `ops::run_auto`, i.e. `Engine::sync_with`: the engine's safety rule applies as for a
//! sync by hand, and a conflict is never decided here.
//!
//! Inputs ([`Input`], through the [`Handle`] every part of the app holds):
//!
//! - folder list or settings changed (commands), sign-in / sign-out, the device-wide
//!   switch ([`cloudit_client::Engine::auto_sync_enabled`]), the global pause
//!   ([`cloudit_client::Engine::paused_until`]: nothing runs until then; it ends by
//!   itself at that time, also checked against the wall clock every [`REWATCH`] in
//!   case the computer slept, and every automatic folder then runs), an app update
//!   about to install (`updater`: no new run starts);
//! - file changes from one watcher per automatic folder (`watcher.rs`); changes while
//!   an operation runs on the folder (or just after: [`BUSY_GRACE`]) are the engine's
//!   own and are dropped;
//! - server events from the follower (`HeadChanged`, `ConflictCreated`, reconnects,
//!   `SignedOut`);
//! - the end of every operation (`ops.rs`): automatic runs feed the retry and conflict
//!   rules, manual ones only update what the planner knows of the folder's state.
//!
//! It also polls the running programs every [`PROCESS_POLL`] while some automatic
//! folder has an associated process (`processes.rs`; while it runs the folder doesn't
//! sync at all), and every [`REWATCH`] retries the watchers of folders whose local path
//! was missing. What the UI shows of it (process running, deferred runs, failures) goes out as the `auto-state` event and is kept
//! in `AppState` for the `auto_states` command. The first failure of a retry streak
//! and a refused token are notified (`notifications.rs`).

mod plan;
mod processes;
mod watcher;

pub use plan::AutoFolderState;
pub use processes::ProcessTable;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cloudit_client::events::RemoteEvent;
use cloudit_client::scan::ProblemKind;
use cloudit_client::{Engine, ErrorKind, LinkedFolder, SyncOutcome};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, MissedTickBehavior};

use crate::commands::auto as settings;
use crate::error::{CommandError, ErrorDetails};
use crate::events::{AUTO_STATE, emit};
use crate::notifications;
use crate::ops::{self, Outcome};
use crate::state::AppState;
use plan::{Ended, FolderCfg, Planner, Trigger};
use watcher::FolderWatcher;

/// How often the running programs are listed (only while some folder needs it).
const PROCESS_POLL: Duration = Duration::from_secs(5);
/// How often watchers that couldn't start (missing folder) are tried again.
const REWATCH: Duration = Duration::from_secs(60);
/// File events this soon after an operation on the folder are its own writes.
pub const BUSY_GRACE: Duration = Duration::from_secs(2);

/// What the scheduler reacts to.
#[derive(Debug)]
pub enum Input {
    /// Something changed in a watched folder.
    Files(i64),
    /// Folders were linked, unlinked or their settings changed.
    FoldersChanged,
    /// Signed in (true) or out.
    Session(bool),
    /// The device-wide switch.
    Enabled(bool),
    /// The global pause: until then, or resumed (`None`).
    Paused(Option<DateTime<Utc>>),
    /// An app update is being installed (`true`) or failed (`false`): no new run
    /// starts meanwhile.
    Updating(bool),
    /// A server event from the follower.
    Remote(RemoteEvent),
    /// An operation on a folder ended (`auto`: started by the scheduler).
    OpFinished {
        folder: i64,
        auto: bool,
        ended: Ended,
    },
}

/// Sends inputs to the scheduler (cheap to clone).
#[derive(Debug, Clone)]
pub struct Handle {
    tx: UnboundedSender<Input>,
}

impl Handle {
    /// Sends an input; dropped when the scheduler is gone (app exit).
    pub fn send(&self, input: Input) {
        let _closed = self.tx.send(input);
    }
}

/// The scheduler's channel: the handle for `AppState`, the receiver for [`start`].
pub fn channel() -> (Handle, UnboundedReceiver<Input>) {
    let (tx, rx) = unbounded_channel();
    (Handle { tx }, rx)
}

/// Starts the scheduler task (once, after `AppState` is managed).
pub fn start(app: &AppHandle, rx: UnboundedReceiver<Input>) {
    let state = app.state::<AppState>();
    let engine = state.engine.clone();
    let tx = state.auto.tx.clone();
    let enabled = engine.auto_sync_enabled().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the automatic sync switch: assuming on");
        true
    });
    let paused_until = engine.paused_until().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the global pause: assuming none");
        None
    });
    let mut planner = Planner::new(engine.signed_in(), enabled);
    planner.set_paused_until(paused_until.map(instant_of), Instant::now());
    let scheduler = Scheduler {
        app: app.clone(),
        planner,
        paused_until,
        engine,
        tx,
        links: HashMap::new(),
        watchers: HashMap::new(),
        watch_failed: HashSet::new(),
        processes: None,
        process_error_logged: false,
        view: Vec::new(),
    };
    tauri::async_runtime::spawn(scheduler.run(rx));
}

/// The monotonic instant of a wall-clock time (now for a time already past).
fn instant_of(at: DateTime<Utc>) -> Instant {
    let left = (at - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    Instant::now() + left
}

/// How an operation's result counts for the scheduler.
pub fn ended_of<T: Outcome>(result: &Result<T, CommandError>) -> Ended {
    match result {
        Ok(outcome) => Ended::Done {
            outcome: outcome
                .report()
                .map_or(plan::Outcome::UpToDate, |r| match r.outcome {
                    SyncOutcome::UpToDate => plan::Outcome::UpToDate,
                    SyncOutcome::Pushed { .. } => plan::Outcome::Pushed,
                    SyncOutcome::Pulled { .. } => plan::Outcome::Pulled,
                    SyncOutcome::NeedsDecision { .. } => plan::Outcome::NeedsDecision,
                    SyncOutcome::Deferred => plan::Outcome::Deferred,
                }),
            state: outcome.status().state,
        },
        Err(error) => Ended::Failed {
            kind: error.kind,
            unreadable_only: unreadable_only(error),
        },
    }
}

/// Only files that couldn't be read (typically locked by the running game) stopped it.
pub fn unreadable_only(error: &CommandError) -> bool {
    error.kind == ErrorKind::ScanProblems
        && matches!(&error.details, Some(ErrorDetails::ScanProblems { problems })
            if !problems.is_empty() && problems.iter().all(|p| p.kind == ProblemKind::Unreadable))
}

struct Scheduler {
    app: AppHandle,
    engine: Engine,
    tx: UnboundedSender<Input>,
    planner: Planner,
    /// The end of the global pause on the wall clock: the planner's monotonic instant
    /// may run late after the computer slept, so this is checked too.
    paused_until: Option<DateTime<Utc>>,
    links: HashMap<i64, LinkedFolder>,
    watchers: HashMap<i64, FolderWatcher>,
    /// Folders whose watcher failed to start (logged once until it works).
    watch_failed: HashSet<i64>,
    processes: Option<ProcessTable>,
    process_error_logged: bool,
    view: Vec<AutoFolderState>,
}

impl Scheduler {
    async fn run(mut self, mut rx: UnboundedReceiver<Input>) {
        self.reload();
        let mut process_tick = tokio::time::interval(PROCESS_POLL);
        process_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut rewatch_tick = tokio::time::interval(REWATCH);
        rewatch_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let next = self.planner.next_due();
            let polling = !self.planner.processes().is_empty();
            let mut retry_failed = false;
            tokio::select! {
                input = rx.recv() => {
                    let Some(input) = input else { break };
                    self.handle(input);
                }
                () = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {}
                _ = process_tick.tick(), if polling => self.poll_processes().await,
                _ = rewatch_tick.tick() => retry_failed = true,
            }
            self.end_pause_if_due();
            self.reconcile_watchers(retry_failed).await;
            if self.planner.process_unknown() {
                self.poll_processes().await;
            }
            self.start_due();
            self.publish();
        }
        tracing::debug!("the automatic sync scheduler ended");
    }

    fn handle(&mut self, input: Input) {
        let now = Instant::now();
        match input {
            Input::Files(folder) => {
                let state = self.app.state::<AppState>();
                if !state.folder_busy(folder, BUSY_GRACE) {
                    self.planner.trigger(folder, Trigger::Files, now);
                }
            }
            Input::FoldersChanged => {
                // A relocation or new settings deserve a new attempt.
                self.watch_failed.clear();
                self.reload();
            }
            Input::Session(signed_in) => {
                if signed_in {
                    self.app.state::<AppState>().alerts(|a| a.signed_in());
                }
                self.reload();
                self.planner.set_session(signed_in, now);
            }
            Input::Enabled(enabled) => self.planner.set_enabled(enabled, now),
            Input::Paused(until) => {
                self.paused_until = until;
                self.planner.set_paused_until(until.map(instant_of), now);
            }
            Input::Updating(updating) => self.planner.set_updating(updating, now),
            Input::Remote(event) => match event {
                RemoteEvent::Connected { .. } => self.planner.connected(now),
                RemoteEvent::HeadChanged { folder, .. }
                | RemoteEvent::ConflictCreated { folder, .. } => {
                    self.planner.trigger(folder, Trigger::Remote, now);
                }
                RemoteEvent::SignedOut { .. } => {
                    self.planner.set_session(false, now);
                    notifications::signed_out(&self.app);
                }
                RemoteEvent::Disconnected { .. }
                | RemoteEvent::FoldersChanged
                | RemoteEvent::UsageChanged => {}
            },
            Input::OpFinished {
                folder,
                auto,
                ended,
            } => {
                if auto {
                    let streak = self.planner.finished(folder, ended, now);
                    if let Ended::Failed { kind, .. } = ended {
                        if matches!(kind, ErrorKind::Unauthorized | ErrorKind::AccountBlocked) {
                            notifications::signed_out(&self.app);
                        } else if streak {
                            tracing::warn!(folder, %kind, "automatic sync failed: retrying later");
                            let notice = self
                                .app
                                .state::<AppState>()
                                .alerts(|a| a.failed(folder, streak));
                            if let Some(notice) = notice {
                                notifications::show(&self.app, notice);
                            }
                        } else {
                            tracing::debug!(folder, %kind, "automatic sync failed again");
                        }
                    }
                } else if let Ended::Done { state, .. } = ended {
                    self.planner.observed(folder, state);
                }
                // A decision may have unpaused the folder.
                self.reload();
            }
        }
    }

    /// Ends the global pause when its time came (by the monotonic clock or the wall
    /// clock): every automatic folder runs, and the UI and the tray hear of it.
    fn end_pause_if_due(&mut self) {
        let now = Instant::now();
        let wall_due = self.paused_until.is_some_and(|until| until <= Utc::now());
        if wall_due {
            self.planner.set_paused_until(None, now);
        }
        let ended = self.planner.expire_pause(now) || wall_due;
        if !ended {
            return;
        }
        self.paused_until = None;
        tracing::info!("the pause of automatic sync ended");
        if let Err(error) = self.engine.set_paused_until(None) {
            // Harmless: a past end reads as no pause.
            tracing::debug!(%error, "could not clear the pause");
        }
        settings::changed(&self.app);
    }

    /// Reads the folder list again (and lets the tray show it).
    fn reload(&mut self) {
        self.app.state::<AppState>().tray.refresh();
        let folders = match self.engine.folders() {
            Ok(folders) => folders,
            Err(error) => {
                tracing::warn!(%error, "automatic sync: could not read the folders");
                return;
            }
        };
        let cfgs: Vec<(i64, FolderCfg)> = folders
            .iter()
            .map(|f| {
                (
                    f.id,
                    FolderCfg {
                        auto: f.auto_sync,
                        paused: f.paused,
                        quiet: Duration::from_secs(u64::from(f.quiet_secs)),
                        process: f.process_name.clone(),
                    },
                )
            })
            .collect();
        self.planner.set_folders(cfgs, Instant::now());
        self.links = folders.into_iter().map(|f| (f.id, f)).collect();
    }

    /// Keeps one watcher per eligible folder whose path exists; a watcher that starts
    /// means a run (the folder may have changed while unwatched).
    async fn reconcile_watchers(&mut self, retry_failed: bool) {
        let eligible: HashSet<i64> = self.planner.eligible_folders().collect();
        let links = &self.links;
        self.watchers.retain(|id, w| {
            eligible.contains(id)
                && links
                    .get(id)
                    .is_some_and(|l| l.local_path == w.path && l.ignore == w.ignore)
                && w.path.is_dir()
        });
        let missing: Vec<(i64, PathBuf, Vec<String>)> = eligible
            .iter()
            .filter(|id| !self.watchers.contains_key(id))
            .filter(|id| retry_failed || !self.watch_failed.contains(id))
            .filter_map(|id| self.links.get(id))
            .filter(|l| l.local_path.is_dir())
            .map(|l| (l.id, l.local_path.clone(), l.ignore.clone()))
            .collect();
        for (id, path, ignore) in missing {
            let tx = self.tx.clone();
            let started = tauri::async_runtime::spawn_blocking(move || {
                FolderWatcher::start(id, &path, &ignore, tx)
            })
            .await
            .map_err(|e| format!("watcher task: {e}"))
            .and_then(|r| r);
            match started {
                Ok(watcher) => {
                    self.watch_failed.remove(&id);
                    self.watchers.insert(id, watcher);
                    self.planner.trigger(id, Trigger::Start, Instant::now());
                }
                Err(error) => {
                    if self.watch_failed.insert(id) {
                        tracing::warn!(folder = id, %error, "could not watch the folder");
                    }
                }
            }
        }
    }

    /// Asks which associated processes run.
    async fn poll_processes(&mut self) {
        let wanted = self.planner.processes();
        if wanted.is_empty() {
            return;
        }
        let names: Vec<String> = wanted.iter().map(|(_, name)| name.clone()).collect();
        let table = self.processes.take();
        let polled = tauri::async_runtime::spawn_blocking(move || {
            let mut table = match table {
                Some(table) => table,
                None => ProcessTable::new()?,
            };
            let running = table.running(&names);
            Ok::<_, String>((table, running))
        })
        .await
        .map_err(|e| format!("process task: {e}"))
        .and_then(|r| r);
        let now = Instant::now();
        match polled {
            Ok((table, running)) => {
                self.processes = Some(table);
                for ((folder, _), running) in wanted.iter().zip(running) {
                    self.planner.process_seen(*folder, running, now);
                }
            }
            Err(error) => {
                if !std::mem::replace(&mut self.process_error_logged, true) {
                    tracing::warn!(%error, "cannot list the running programs: ignoring them");
                }
                for (folder, _) in &wanted {
                    self.planner.process_seen(*folder, false, now);
                }
            }
        }
    }

    fn start_due(&mut self) {
        for run in self.planner.take_due(Instant::now()) {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                // The outcome reaches the planner through `OpFinished`.
                let _ = ops::run_auto(&app, run.folder).await;
            });
        }
    }

    /// Sends the UI view when it changed.
    fn publish(&mut self) {
        let view = self.planner.view();
        if view != self.view {
            self.app.state::<AppState>().set_auto_view(view.clone());
            emit(&self.app, AUTO_STATE, &view);
            self.view = view;
        }
    }
}
