//! The scheduler's decisions, pure: given the folders' settings, what happened (a
//! trigger, a process seen or gone, a run that ended) and the time, when each folder
//! runs next and whether it may pull. No I/O, no clocks of its own: the caller passes
//! `now` (a tokio `Instant`, so tests run on paused time).
//!
//! Rules (STRUCTURE.md "Automatic sync"):
//!
//! - a folder runs only while [`Planner::active`] (signed in, the device-wide switch
//!   on, no global pause, no update installing, a compatible server: the single check
//!   point) and it is eligible (`auto_sync`
//!   on, not paused);
//! - while paused nothing fires; the pause ends at its time ([`Planner::next_due`]
//!   wakes the caller then, [`Planner::expire_pause`] ends it) and every eligible
//!   folder runs at once, like at a start;
//! - file changes reset a debounce: the run starts `quiet` after the last one; they are
//!   ignored while this device's conflict decision is open (the folder would only
//!   collect more conflict snapshots);
//! - start, sign-in, a new eligible folder and server events run at once; the exit of
//!   the associated process after [`SETTLE`];
//! - while that process runs the folder doesn't sync at all (a game such as Minecraft
//!   writes its files constantly: every write would become an upload); what came due
//!   meanwhile waits, and the exit runs it. A folder whose process state is still
//!   unknown waits for the first poll;
//! - a failure retries with [`BACKOFF`] (and on any trigger); a refused token stops
//!   everything until the next sign-in, an incompatible server (`app_outdated` /
//!   `server_outdated`) until it is compatible again ([`Planner::set_compatible`]); files that can't be read (a running game
//!   locks them) retry quietly, after the process exits when there is one;
//! - a conflict is never retried by itself: it waits for the user or a new trigger.

use std::collections::BTreeMap;
use std::time::Duration;

use cloudit_client::{ErrorKind, SyncState};
use serde::Serialize;
use tokio::time::Instant;

/// Delay after the associated process exits (it may still be writing).
pub const SETTLE: Duration = Duration::from_secs(3);
/// Retry delays after consecutive failures (the last one repeats).
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(300),
    Duration::from_secs(900),
];
/// Retry delay when only unreadable (locked) files stopped the run.
pub const UNREADABLE_RETRY: Duration = Duration::from_secs(60);

/// A folder's settings, as the planner needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCfg {
    pub auto: bool,
    pub paused: bool,
    pub quiet: Duration,
    pub process: Option<String>,
}

/// Why a folder should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// App start, sign-in, the folder just became automatic, its watcher (re)started.
    Start,
    /// Something changed in the folder (debounced).
    Files,
    /// The server moved the head or a conflict appeared (or the stream reconnected).
    Remote,
}

/// How an automatic run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The run returned a report.
    Done { outcome: Outcome, state: SyncState },
    /// The run failed; `unreadable_only`: only files that couldn't be read stopped it.
    Failed {
        kind: ErrorKind,
        unreadable_only: bool,
    },
}

/// The kind of `SyncOutcome` (without its data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Pushed,
    Pulled,
    NeedsDecision,
    Deferred,
}

/// A run to start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub folder: i64,
}

/// What the UI shows of a folder's automatic sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct AutoFolderState {
    pub folder: i64,
    /// The associated process is running (nothing is synced meanwhile).
    pub process_running: bool,
    /// The last run had to leave a pull or a decision for later.
    pub deferred: bool,
    /// Consecutive failed runs (retried with backoff).
    pub failures: u32,
}

#[derive(Debug, Clone)]
struct Slot {
    cfg: FolderCfg,
    due: Option<Instant>,
    running: bool,
    failures: u32,
    /// `None` until the first poll after the process name was set.
    process_running: Option<bool>,
    deferred: bool,
    /// This device's conflict decision is open.
    hold: bool,
}

impl Slot {
    fn new(cfg: FolderCfg) -> Self {
        Self {
            cfg,
            due: None,
            running: false,
            failures: 0,
            process_running: None,
            deferred: false,
            hold: false,
        }
    }

    fn eligible(&self) -> bool {
        self.cfg.auto && !self.cfg.paused
    }

    /// Its process runs, or hasn't been polled yet: what is due waits.
    fn held_by_process(&self) -> bool {
        self.cfg.process.is_some() && self.process_running != Some(false)
    }

    /// Runs no later than `at`.
    fn due_by(&mut self, at: Instant) {
        self.due = Some(self.due.map_or(at, |due| due.min(at)));
    }
}

/// The decisions (see the module docs).
#[derive(Debug, Default)]
pub struct Planner {
    slots: BTreeMap<i64, Slot>,
    signed_in: bool,
    enabled: bool,
    /// The global pause ends then (`None`: not paused).
    paused_until: Option<Instant>,
    /// An app update is about to be installed: no new run starts (those running end).
    updating: bool,
    /// The server doesn't work with this app (`app_outdated` / `server_outdated`).
    incompatible: bool,
    /// The server stream connected at least once in this session (later `Connected`
    /// events are reconnects: changes may have been missed).
    connected: bool,
}

impl Planner {
    #[must_use]
    pub fn new(signed_in: bool, enabled: bool) -> Self {
        Self {
            signed_in,
            enabled,
            ..Self::default()
        }
    }

    /// Whether automatic sync may run at all: the one check point.
    #[must_use]
    pub fn active(&self) -> bool {
        self.signed_in
            && self.enabled
            && self.paused_until.is_none()
            && !self.updating
            && !self.incompatible
    }

    /// Stops every automatic run while the server and this app don't work together
    /// (`true`: they do again, every eligible folder runs at once).
    pub fn set_compatible(&mut self, compatible: bool, now: Instant) {
        self.update_active(now, |p| p.incompatible = !compatible);
    }

    /// Holds every automatic run while an app update installs (`false`: it failed,
    /// runs resume at once).
    pub fn set_updating(&mut self, updating: bool, now: Instant) {
        self.update_active(now, |p| p.updating = updating);
    }

    /// Pauses every automatic run until `until`, or resumes (`None`, or a time already
    /// past). Resuming runs every eligible folder at once.
    pub fn set_paused_until(&mut self, until: Option<Instant>, now: Instant) {
        let until = until.filter(|until| *until > now);
        self.update_active(now, |p| p.paused_until = until);
    }

    /// Ends the pause if its time came; whether it ended now.
    pub fn expire_pause(&mut self, now: Instant) -> bool {
        if self.paused_until.is_some_and(|until| until <= now) {
            self.set_paused_until(None, now);
            true
        } else {
            false
        }
    }

    fn eligible(&self, folder: i64) -> bool {
        self.active() && self.slots.get(&folder).is_some_and(Slot::eligible)
    }

    /// The eligible folders.
    pub fn eligible_folders(&self) -> impl Iterator<Item = i64> + '_ {
        self.slots
            .iter()
            .filter(|(_, s)| self.active() && s.eligible())
            .map(|(id, _)| *id)
    }

    /// Eligible folders with an associated process: `(folder, name)`.
    #[must_use]
    pub fn processes(&self) -> Vec<(i64, String)> {
        self.slots
            .iter()
            .filter(|(_, s)| self.active() && s.eligible())
            .filter_map(|(id, s)| s.cfg.process.clone().map(|p| (*id, p)))
            .collect()
    }

    /// Some eligible folder waits for its first process poll.
    #[must_use]
    pub fn process_unknown(&self) -> bool {
        self.slots.values().any(|s| {
            self.active() && s.eligible() && s.cfg.process.is_some() && s.process_running.is_none()
        })
    }

    pub fn set_session(&mut self, signed_in: bool, now: Instant) {
        self.connected = false;
        self.set_active(signed_in, self.enabled, now);
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.set_active(self.signed_in, enabled, now);
    }

    fn set_active(&mut self, signed_in: bool, enabled: bool, now: Instant) {
        self.update_active(now, |p| {
            p.signed_in = signed_in;
            p.enabled = enabled;
        });
    }

    /// Applies a change to what `active` depends on: becoming active runs every
    /// eligible folder, becoming inactive forgets what was due.
    fn update_active(&mut self, now: Instant, change: impl FnOnce(&mut Self)) {
        let was = self.active();
        change(self);
        let active = self.active();
        for slot in self.slots.values_mut() {
            if active && !was {
                slot.due_by(now);
            } else if !active {
                slot.due = None;
                slot.failures = 0;
            }
        }
    }

    /// Replaces the folder list: new or newly eligible folders run at once.
    pub fn set_folders(
        &mut self,
        folders: impl IntoIterator<Item = (i64, FolderCfg)>,
        now: Instant,
    ) {
        let mut next = BTreeMap::new();
        for (id, cfg) in folders {
            let slot = match self.slots.remove(&id) {
                Some(mut slot) => {
                    let was = slot.eligible();
                    if slot.cfg.process != cfg.process {
                        slot.process_running = None;
                        slot.deferred = false;
                    }
                    slot.cfg = cfg;
                    if !slot.eligible() {
                        slot.due = None;
                        slot.failures = 0;
                    } else if !was {
                        slot.due_by(now);
                    }
                    slot
                }
                None => {
                    let mut slot = Slot::new(cfg);
                    if slot.eligible() {
                        slot.due_by(now);
                    }
                    slot
                }
            };
            next.insert(id, slot);
        }
        self.slots = next;
    }

    /// Something happened that calls for a run of `folder`.
    pub fn trigger(&mut self, folder: i64, trigger: Trigger, now: Instant) {
        if !self.eligible(folder) {
            return;
        }
        let Some(slot) = self.slots.get_mut(&folder) else {
            return;
        };
        match trigger {
            Trigger::Files if slot.hold => {}
            Trigger::Files => slot.due = Some(now + slot.cfg.quiet),
            Trigger::Start | Trigger::Remote => slot.due_by(now),
        }
    }

    /// Every eligible folder should run (reconnect, switch turned on).
    pub fn trigger_all(&mut self, trigger: Trigger, now: Instant) {
        let ids: Vec<i64> = self.eligible_folders().collect();
        for id in ids {
            self.trigger(id, trigger, now);
        }
    }

    /// The server stream connected: from the second time on, run everything.
    pub fn connected(&mut self, now: Instant) {
        if std::mem::replace(&mut self.connected, true) {
            self.trigger_all(Trigger::Remote, now);
        }
    }

    /// A process poll's answer for `folder`'s associated process.
    pub fn process_seen(&mut self, folder: i64, running: bool, now: Instant) {
        let active = self.active();
        let Some(slot) = self.slots.get_mut(&folder) else {
            return;
        };
        let was = slot.process_running.replace(running);
        if was == Some(true) && !running && active && slot.eligible() {
            // Not sooner than SETTLE, even if something came due while it ran: the
            // program may still be writing.
            let at = now + SETTLE;
            slot.due = Some(slot.due.map_or(at, |due| due.max(at)));
        }
    }

    /// The runs to start now (marked running).
    pub fn take_due(&mut self, now: Instant) -> Vec<Run> {
        if !self.active() {
            return Vec::new();
        }
        let mut runs = Vec::new();
        for (id, slot) in &mut self.slots {
            if !slot.eligible() || slot.running || slot.held_by_process() {
                continue;
            }
            if slot.due.is_some_and(|due| due <= now) {
                slot.due = None;
                slot.running = true;
                runs.push(Run { folder: *id });
            }
        }
        runs
    }

    /// When the next run is due (`None`: nothing scheduled). While paused (and
    /// otherwise active) that is the end of the pause.
    #[must_use]
    pub fn next_due(&self) -> Option<Instant> {
        if !self.active() {
            return self.paused_until.filter(|_| self.signed_in && self.enabled);
        }
        self.slots
            .values()
            .filter(|s| s.eligible() && !s.running && !s.held_by_process())
            .filter_map(|s| s.due)
            .min()
    }

    /// An automatic run ended; whether this failure starts a streak (log it at warn).
    pub fn finished(&mut self, folder: i64, ended: Ended, now: Instant) -> bool {
        if let Some(slot) = self.slots.get_mut(&folder) {
            slot.running = false;
        }
        if let Ended::Failed {
            kind: ErrorKind::Unauthorized | ErrorKind::AccountBlocked | ErrorKind::NotSignedIn,
            ..
        } = ended
        {
            self.set_session(false, now);
            return false;
        }
        if let Ended::Failed {
            kind: ErrorKind::AppOutdated | ErrorKind::ServerOutdated,
            ..
        } = ended
        {
            // No retries: nothing works until the app or the server is updated.
            self.set_compatible(false, now);
            return false;
        }
        let Some(slot) = self.slots.get_mut(&folder) else {
            return false;
        };
        match ended {
            Ended::Done { outcome, state } => {
                slot.failures = 0;
                slot.hold = state == SyncState::Conflict;
                slot.deferred = outcome == Outcome::Deferred;
                // Changes made while it ran (their file events were ignored, or the
                // head moved again): one more run.
                if matches!(outcome, Outcome::Pushed | Outcome::Pulled) && slot.due.is_none() {
                    match state {
                        SyncState::LocalChanges => slot.due = Some(now + slot.cfg.quiet),
                        SyncState::RemoteChanges => slot.due = Some(now),
                        _ => {}
                    }
                }
                false
            }
            Ended::Failed {
                kind,
                unreadable_only,
            } => match kind {
                ErrorKind::Cancelled | ErrorKind::Paused | ErrorKind::NotLinked => false,
                ErrorKind::ScanProblems if unreadable_only => {
                    if slot.process_running == Some(true) {
                        slot.deferred = true;
                    } else {
                        slot.due_by(now + UNREADABLE_RETRY);
                    }
                    false
                }
                _ => {
                    slot.failures = slot.failures.saturating_add(1);
                    let step = usize::try_from(slot.failures - 1).unwrap_or(usize::MAX);
                    let delay = BACKOFF[step.min(BACKOFF.len() - 1)];
                    slot.due_by(now + delay);
                    slot.failures == 1
                }
            },
        }
    }

    /// Another operation (by hand) reported the folder's state.
    pub fn observed(&mut self, folder: i64, state: SyncState) {
        if let Some(slot) = self.slots.get_mut(&folder) {
            slot.hold = state == SyncState::Conflict;
            if state != SyncState::RemoteChanges && state != SyncState::Conflict {
                slot.deferred = false;
            }
        }
    }

    /// The UI view of every eligible folder.
    #[must_use]
    pub fn view(&self) -> Vec<AutoFolderState> {
        self.slots
            .iter()
            .filter(|(_, s)| self.active() && s.eligible())
            .map(|(id, s)| AutoFolderState {
                folder: *id,
                process_running: s.process_running == Some(true),
                deferred: s.deferred,
                failures: s.failures,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::advance;

    const QUIET: Duration = Duration::from_secs(30);

    fn cfg(process: Option<&str>) -> FolderCfg {
        FolderCfg {
            auto: true,
            paused: false,
            quiet: QUIET,
            process: process.map(str::to_owned),
        }
    }

    /// Signed in, switch on, folder 1 automatic; its start run already taken.
    fn planner(process: Option<&str>) -> Planner {
        let mut p = Planner::new(true, true);
        p.set_folders([(1, cfg(process))], Instant::now());
        if process.is_some() {
            p.process_seen(1, false, Instant::now());
        }
        assert_eq!(p.take_due(Instant::now()).len(), 1);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        p
    }

    fn done(outcome: Outcome, state: SyncState) -> Ended {
        Ended::Done { outcome, state }
    }

    fn failed(kind: ErrorKind) -> Ended {
        Ended::Failed {
            kind,
            unreadable_only: false,
        }
    }

    fn due_now(p: &mut Planner) -> Vec<Run> {
        p.take_due(Instant::now())
    }

    #[tokio::test(start_paused = true)]
    async fn file_changes_are_debounced() {
        let mut p = planner(None);
        p.trigger(1, Trigger::Files, Instant::now());
        advance(Duration::from_secs(20)).await;
        assert!(due_now(&mut p).is_empty());
        // Another change resets the quiet period.
        p.trigger(1, Trigger::Files, Instant::now());
        advance(Duration::from_secs(20)).await;
        assert!(due_now(&mut p).is_empty());
        assert_eq!(p.next_due(), Some(Instant::now() + Duration::from_secs(10)));
        advance(Duration::from_secs(10)).await;
        assert_eq!(due_now(&mut p), vec![Run { folder: 1 }]);
        // Running: not started twice.
        p.trigger(1, Trigger::Remote, Instant::now());
        assert!(due_now(&mut p).is_empty());
        p.finished(1, done(Outcome::Pushed, SyncState::InSync), Instant::now());
        // The trigger that came while it ran still runs.
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn starts_and_remote_events_run_at_once() {
        let mut p = Planner::new(true, true);
        p.set_folders([(1, cfg(None)), (2, cfg(None))], Instant::now());
        assert_eq!(due_now(&mut p).len(), 2);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        p.finished(
            2,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        p.trigger(2, Trigger::Remote, Instant::now());
        assert_eq!(due_now(&mut p)[0].folder, 2);
        p.finished(2, done(Outcome::Pulled, SyncState::InSync), Instant::now());
        // The first connection is the start; a reconnect runs everything.
        p.connected(Instant::now());
        assert!(due_now(&mut p).is_empty());
        p.connected(Instant::now());
        assert_eq!(due_now(&mut p).len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn only_eligible_folders_run_while_active() {
        let mut p = Planner::new(true, false);
        let manual = FolderCfg {
            auto: false,
            ..cfg(None)
        };
        let paused = FolderCfg {
            paused: true,
            ..cfg(None)
        };
        p.set_folders(
            [(1, cfg(None)), (2, manual), (3, paused.clone())],
            Instant::now(),
        );
        // The switch is off.
        assert!(due_now(&mut p).is_empty());
        assert_eq!(p.next_due(), None);
        p.set_enabled(true, Instant::now());
        assert_eq!(due_now(&mut p), vec![Run { folder: 1 }]);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        for id in [1, 2, 3] {
            p.trigger(id, Trigger::Remote, Instant::now());
        }
        assert_eq!(due_now(&mut p).len(), 1);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        // Unpausing makes folder 3 run.
        p.set_folders(
            [
                (1, cfg(None)),
                (
                    3,
                    FolderCfg {
                        paused: false,
                        ..paused
                    },
                ),
            ],
            Instant::now(),
        );
        assert_eq!(due_now(&mut p)[0].folder, 3);
        // Signed out: nothing, even when triggered.
        p.finished(
            3,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        p.set_session(false, Instant::now());
        p.trigger(1, Trigger::Remote, Instant::now());
        assert!(due_now(&mut p).is_empty());
        assert!(p.view().is_empty());
        p.set_session(true, Instant::now());
        assert_eq!(due_now(&mut p).len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_app_update_holds_every_run() {
        let mut p = planner(None);
        p.set_updating(true, Instant::now());
        assert!(!p.active());
        p.trigger(1, Trigger::Remote, Instant::now());
        assert!(due_now(&mut p).is_empty());
        assert_eq!(p.next_due(), None);
        // The update failed: everything runs again at once.
        p.set_updating(false, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_global_pause_stops_everything_until_it_ends() {
        let mut p = planner(None);
        let hour = Duration::from_secs(3600);
        p.trigger(1, Trigger::Files, Instant::now());
        p.set_paused_until(Some(Instant::now() + hour), Instant::now());
        assert!(!p.active());
        // What was due is forgotten and nothing fires meanwhile; the caller wakes at
        // the end of the pause.
        assert_eq!(p.next_due(), Some(Instant::now() + hour));
        p.trigger(1, Trigger::Remote, Instant::now());
        p.connected(Instant::now());
        p.connected(Instant::now());
        p.process_seen(1, false, Instant::now());
        advance(QUIET).await;
        assert!(due_now(&mut p).is_empty());
        assert!(p.view().is_empty());
        assert!(!p.expire_pause(Instant::now()));
        advance(hour - QUIET).await;
        // It ends by itself and runs every automatic folder.
        assert!(p.expire_pause(Instant::now()));
        assert!(p.active());
        assert_eq!(due_now(&mut p), vec![Run { folder: 1 }]);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        // Resuming by hand runs at once too; an end already past is no pause.
        p.set_paused_until(Some(Instant::now() + hour), Instant::now());
        p.set_paused_until(None, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::InSync),
            Instant::now(),
        );
        p.set_paused_until(Some(Instant::now()), Instant::now());
        assert!(p.active());
        // Signed out while paused: no wake-up for the pause either.
        p.set_paused_until(Some(Instant::now() + hour), Instant::now());
        p.set_session(false, Instant::now());
        assert_eq!(p.next_due(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_process_blocks_every_run_and_its_exit_runs() {
        let mut p = Planner::new(true, true);
        p.set_folders([(1, cfg(Some("game.exe")))], Instant::now());
        // Waits for the first poll, without a wake-up for the start run.
        assert!(p.process_unknown());
        assert!(due_now(&mut p).is_empty());
        assert_eq!(p.next_due(), None);
        p.process_seen(1, true, Instant::now());
        assert!(due_now(&mut p).is_empty());
        assert!(p.view()[0].process_running);
        // While it runs nothing syncs: neither its writes nor server changes.
        p.trigger(1, Trigger::Files, Instant::now());
        p.trigger(1, Trigger::Remote, Instant::now());
        advance(QUIET * 10).await;
        p.process_seen(1, true, Instant::now());
        assert!(due_now(&mut p).is_empty());
        assert_eq!(p.next_due(), None);
        // Its exit runs once, SETTLE later, though something was due long before.
        p.process_seen(1, false, Instant::now());
        assert_eq!(p.next_due(), Some(Instant::now() + SETTLE));
        advance(SETTLE - Duration::from_millis(1)).await;
        assert!(due_now(&mut p).is_empty());
        advance(Duration::from_millis(1)).await;
        assert_eq!(due_now(&mut p), vec![Run { folder: 1 }]);
        p.finished(1, done(Outcome::Pushed, SyncState::InSync), Instant::now());
        assert_eq!(p.next_due(), None);
        assert!(!p.view()[0].process_running);
        // Without the program, file changes run as usual.
        p.trigger(1, Trigger::Files, Instant::now());
        advance(QUIET).await;
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_program_started_during_a_run_holds_the_next_one() {
        let mut p = planner(Some("game"));
        p.trigger(1, Trigger::Remote, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
        p.process_seen(1, true, Instant::now());
        p.finished(
            1,
            done(Outcome::Pushed, SyncState::LocalChanges),
            Instant::now(),
        );
        // The follow-up run waits for the exit.
        advance(QUIET).await;
        assert!(due_now(&mut p).is_empty());
        p.process_seen(1, false, Instant::now());
        advance(SETTLE).await;
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_back_off_and_log_once() {
        let mut p = planner(None);
        p.trigger(1, Trigger::Remote, Instant::now());
        let mut warned = Vec::new();
        for delay in [60, 120, 300, 900, 900] {
            assert_eq!(due_now(&mut p).len(), 1);
            warned.push(p.finished(1, failed(ErrorKind::Network), Instant::now()));
            advance(Duration::from_secs(delay - 1)).await;
            assert!(due_now(&mut p).is_empty(), "{delay}");
            advance(Duration::from_secs(1)).await;
        }
        assert_eq!(warned, vec![true, false, false, false, false]);
        assert_eq!(p.view()[0].failures, 5);
        // A trigger retries sooner; success ends the streak.
        assert_eq!(due_now(&mut p).len(), 1);
        p.finished(1, failed(ErrorKind::Server), Instant::now());
        p.trigger(1, Trigger::Remote, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
        assert!(!p.finished(1, done(Outcome::Pushed, SyncState::InSync), Instant::now()));
        assert_eq!(p.view()[0].failures, 0);
        p.trigger(1, Trigger::Remote, Instant::now());
        due_now(&mut p);
        assert!(p.finished(1, failed(ErrorKind::Network), Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_token_stops_everything() {
        let mut p = planner(None);
        p.trigger(1, Trigger::Remote, Instant::now());
        due_now(&mut p);
        p.finished(1, failed(ErrorKind::Unauthorized), Instant::now());
        assert!(!p.active());
        advance(Duration::from_secs(3600)).await;
        p.trigger(1, Trigger::Files, Instant::now());
        assert!(due_now(&mut p).is_empty());
        p.set_session(true, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_incompatible_server_stops_everything_without_retries() {
        for kind in [ErrorKind::AppOutdated, ErrorKind::ServerOutdated] {
            let mut p = planner(None);
            p.trigger(1, Trigger::Remote, Instant::now());
            due_now(&mut p);
            // Not a failure streak to log or notify.
            assert!(!p.finished(1, failed(kind), Instant::now()));
            assert!(!p.active());
            assert_eq!(p.next_due(), None);
            advance(Duration::from_secs(3600)).await;
            p.trigger(1, Trigger::Files, Instant::now());
            p.trigger(1, Trigger::Remote, Instant::now());
            assert!(due_now(&mut p).is_empty());
            // Compatible again (the follower re-checked): everything runs at once.
            p.set_compatible(true, Instant::now());
            assert!(p.active());
            assert_eq!(due_now(&mut p).len(), 1);
        }
        // Learned from the follower before any run failed.
        let mut p = planner(None);
        p.set_compatible(false, Instant::now());
        p.trigger(1, Trigger::Remote, Instant::now());
        assert!(due_now(&mut p).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn locked_files_retry_quietly() {
        let mut p = planner(Some("game"));
        p.trigger(1, Trigger::Remote, Instant::now());
        due_now(&mut p);
        let locked = Ended::Failed {
            kind: ErrorKind::ScanProblems,
            unreadable_only: true,
        };
        assert!(!p.finished(1, locked, Instant::now()));
        assert_eq!(p.next_due(), Some(Instant::now() + UNREADABLE_RETRY));
        assert_eq!(p.view()[0].failures, 0);
        // The game started during the run: wait for its exit instead.
        advance(UNREADABLE_RETRY).await;
        assert_eq!(due_now(&mut p).len(), 1);
        p.process_seen(1, true, Instant::now());
        p.finished(1, locked, Instant::now());
        assert_eq!(p.next_due(), None);
        assert!(p.view()[0].deferred);
        p.process_seen(1, false, Instant::now());
        advance(SETTLE).await;
        assert_eq!(due_now(&mut p).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn conflicts_wait_for_the_user() {
        let mut p = planner(None);
        p.trigger(1, Trigger::Files, Instant::now());
        advance(QUIET).await;
        due_now(&mut p);
        p.finished(
            1,
            done(Outcome::NeedsDecision, SyncState::Conflict),
            Instant::now(),
        );
        // Never retried by itself, and more file changes don't pile up conflicts.
        assert_eq!(p.next_due(), None);
        p.trigger(1, Trigger::Files, Instant::now());
        assert_eq!(p.next_due(), None);
        // A server event (decided elsewhere) still runs.
        p.trigger(1, Trigger::Remote, Instant::now());
        assert_eq!(due_now(&mut p).len(), 1);
        p.finished(
            1,
            done(Outcome::NeedsDecision, SyncState::Conflict),
            Instant::now(),
        );
        // Decided by hand: file changes count again.
        p.observed(1, SyncState::InSync);
        p.trigger(1, Trigger::Files, Instant::now());
        assert!(p.next_due().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn changes_made_during_a_run_run_again() {
        let mut p = planner(None);
        p.trigger(1, Trigger::Remote, Instant::now());
        due_now(&mut p);
        p.finished(
            1,
            done(Outcome::Pushed, SyncState::LocalChanges),
            Instant::now(),
        );
        assert_eq!(p.next_due(), Some(Instant::now() + QUIET));
        advance(QUIET).await;
        due_now(&mut p);
        p.finished(
            1,
            done(Outcome::Pulled, SyncState::RemoteChanges),
            Instant::now(),
        );
        assert_eq!(p.next_due(), Some(Instant::now()));
        due_now(&mut p);
        // Nothing done and still changes: no loop.
        p.finished(
            1,
            done(Outcome::UpToDate, SyncState::RemoteChanges),
            Instant::now(),
        );
        assert_eq!(p.next_due(), None);
    }
}
