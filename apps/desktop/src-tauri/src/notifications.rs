//! Desktop notifications, sent by the backend only (the web view has no notification
//! permission), and only for what needs the user: never for a sync that went well.
//!
//! - a conflict: a folder entering `conflict` (this device must decide: an automatic
//!   run that ended in `NeedsDecision`, or a status after a `ConflictCreated` event)
//!   or `server_conflict` (left by another device), once per entry into the state;
//!   the states known at startup count as already notified;
//! - the first failure of an automatic retry streak (the scheduler says which one; the
//!   quiet "files locked by the running program" case never starts a streak);
//! - "sign in again" when the server refuses the token, once until the next sign-in.
//!
//! The device-wide switch (`Engine::notifications_enabled`, default on) silences them,
//! and nothing is shown while the main window is visible and focused (the UI already
//! shows it). The one-time "still running in the tray" notice ignores the switch. The
//! dedupe rules are the pure [`Alerts`].

use std::collections::HashMap;

use cloudit_client::SyncState;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt as _;

use crate::i18n::{Msg, text};
use crate::state::AppState;
use crate::window;

/// Something to tell the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// A folder entered `conflict` (`server: false`) or `server_conflict`.
    Conflict { folder: i64, server: bool },
    /// Automatic sync of a folder started failing.
    SyncFailed { folder: i64 },
    /// The server refused this device's token.
    SignIn,
}

/// The dedupe rules (see the module docs).
#[derive(Debug, Default)]
pub struct Alerts {
    /// Folders in a conflict state already notified (`true`: server conflict).
    conflicts: HashMap<i64, bool>,
    signed_out: bool,
}

impl Alerts {
    /// Starts from the folders' last known states: a conflict that was there before
    /// the app started is not news.
    pub fn new(states: impl IntoIterator<Item = (i64, Option<SyncState>)>) -> Self {
        let mut alerts = Self::default();
        for (folder, state) in states {
            if let Some(state) = state {
                let _ = alerts.observe(folder, state);
            }
        }
        alerts
    }

    /// A folder's state was computed; a notice when it just entered a conflict state.
    pub fn observe(&mut self, folder: i64, state: SyncState) -> Option<Notice> {
        let server = match state {
            SyncState::Conflict => false,
            SyncState::ServerConflict => true,
            _ => {
                self.conflicts.remove(&folder);
                return None;
            }
        };
        let previous = self.conflicts.insert(folder, server);
        (previous != Some(server)).then_some(Notice::Conflict { folder, server })
    }

    /// An automatic run failed; `streak_start` is the scheduler's "first failure".
    #[must_use]
    pub fn failed(&self, folder: i64, streak_start: bool) -> Option<Notice> {
        streak_start.then_some(Notice::SyncFailed { folder })
    }

    /// The server refused the token: a notice the first time.
    pub fn signed_out(&mut self) -> Option<Notice> {
        (!std::mem::replace(&mut self.signed_out, true)).then_some(Notice::SignIn)
    }

    /// Signed in again: a later refusal is news again.
    pub fn signed_in(&mut self) {
        self.signed_out = false;
    }
}

/// Shows `notice` if the switch is on and the window isn't in front.
pub fn show(app: &AppHandle, notice: Notice) {
    let state = app.state::<AppState>();
    match state.engine.notifications_enabled() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(%error, "could not read the notifications switch");
            return;
        }
    }
    if window::in_front(app) {
        return;
    }
    let lang = state.lang();
    let name = |folder: i64| {
        state
            .engine
            .folder(folder)
            .map(|f| f.folder_name)
            .unwrap_or_default()
    };
    let (title, body) = match notice {
        Notice::Conflict { folder, server } => (
            text(lang, Msg::ConflictTitle(&name(folder))),
            text(
                lang,
                if server {
                    Msg::ServerConflictBody
                } else {
                    Msg::ConflictBody
                },
            ),
        ),
        Notice::SyncFailed { folder } => (
            text(lang, Msg::SyncFailedTitle(&name(folder))),
            text(lang, Msg::SyncFailedBody),
        ),
        Notice::SignIn => (text(lang, Msg::SignInTitle), text(lang, Msg::SignInBody)),
    };
    send(app, &title, &body);
}

/// Shows a notification (a failure is only logged: notifications are a courtesy).
pub fn send(app: &AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(%error, "could not show a notification");
    }
}

/// A folder's state was computed (any operation).
pub fn observe(app: &AppHandle, folder: i64, state: SyncState) {
    let notice = app.state::<AppState>().alerts(|a| a.observe(folder, state));
    if let Some(notice) = notice {
        show(app, notice);
    }
}

/// The server refused the token (automatic run or the follower).
pub fn signed_out(app: &AppHandle) {
    let notice = app.state::<AppState>().alerts(Alerts::signed_out);
    if let Some(notice) = notice {
        show(app, notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicts_are_notified_once_per_entry() {
        let mut a = Alerts::new([
            (1, Some(SyncState::Conflict)),
            (2, Some(SyncState::InSync)),
            (3, None),
        ]);
        // Known at startup: not news.
        assert_eq!(a.observe(1, SyncState::Conflict), None);
        assert_eq!(
            a.observe(2, SyncState::Conflict),
            Some(Notice::Conflict {
                folder: 2,
                server: false
            })
        );
        assert_eq!(a.observe(2, SyncState::Conflict), None);
        // Another kind of conflict is news; leaving and entering again too.
        assert_eq!(
            a.observe(2, SyncState::ServerConflict),
            Some(Notice::Conflict {
                folder: 2,
                server: true
            })
        );
        assert_eq!(a.observe(2, SyncState::InSync), None);
        assert!(a.observe(2, SyncState::ServerConflict).is_some());
        assert_eq!(a.observe(3, SyncState::LocalChanges), None);
    }

    #[test]
    fn failures_and_sign_in() {
        let mut a = Alerts::default();
        assert_eq!(a.failed(4, true), Some(Notice::SyncFailed { folder: 4 }));
        assert_eq!(a.failed(4, false), None);
        assert_eq!(a.signed_out(), Some(Notice::SignIn));
        assert_eq!(a.signed_out(), None);
        a.signed_in();
        assert_eq!(a.signed_out(), Some(Notice::SignIn));
    }
}
