//! What the updater shows, as a pure state machine (tested without a network): the
//! phases of a check and of an install the user asked for, and how this build may be
//! updated at all ([`Install`]).

use serde::Serialize;

/// How this build can be updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum Install {
    /// Downloaded, verified and installed in the app (Windows installer, AppImage).
    InApp,
    /// Only a link to the release page (a `.deb` install: the package manager owns
    /// the files).
    ReleasePage,
    /// A development build: updates are shown, never installed.
    Never,
}

impl Install {
    /// The mode for a build: `debug` (dev builds), on Windows, or from an AppImage.
    #[must_use]
    pub fn detect(debug: bool, windows: bool, appimage: bool) -> Self {
        if debug {
            Self::Never
        } else if windows || appimage {
            Self::InApp
        } else {
            Self::ReleasePage
        }
    }
}

/// Where the updater is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum UpdatePhase {
    /// Not checked yet.
    Idle,
    Checking,
    /// The last check found nothing newer.
    UpToDate,
    /// A newer version is out (the user decides).
    Available,
    /// The user asked to update: waiting for running syncs to end.
    Waiting,
    Downloading,
    /// Verified; the installer runs and the app restarts.
    Installing,
    /// The install failed (`error`); the user may try again.
    Failed,
}

/// `update-state` payload and `update_state` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateState {
    pub phase: UpdatePhase,
    pub install: Install,
    /// The running version.
    pub current_version: String,
    /// The newer version found (kept through download, install and failure).
    pub latest_version: Option<String>,
    /// Its release notes (`latest.json` `notes`).
    pub notes: Option<String>,
    /// Its release page.
    pub release_url: Option<String>,
    /// Bytes downloaded so far, and the total when known.
    pub downloaded: u64,
    pub total: Option<u64>,
    /// English reason of the last failure (check or install), for the UI and logs.
    pub error: Option<String>,
    /// When the last check finished (RFC 3339).
    pub checked_at: Option<String>,
}

/// The release page of a version.
#[must_use]
pub fn release_url(version: &str) -> String {
    format!("https://github.com/jacopofilonzi/cloudit/releases/tag/v{version}")
}

impl UpdateState {
    #[must_use]
    pub fn new(current_version: String, install: Install) -> Self {
        Self {
            phase: UpdatePhase::Idle,
            install,
            current_version,
            latest_version: None,
            notes: None,
            release_url: None,
            downloaded: 0,
            total: None,
            error: None,
            checked_at: None,
        }
    }

    /// An install is under way (checks are skipped, a second request ignored).
    #[must_use]
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Waiting | UpdatePhase::Downloading | UpdatePhase::Installing
        )
    }

    /// A check may start now.
    #[must_use]
    pub fn may_check(&self) -> bool {
        !self.busy() && self.phase != UpdatePhase::Checking
    }

    pub fn checking(&mut self) {
        self.phase = UpdatePhase::Checking;
    }

    /// A check ended: the newer version and its notes, or `None`.
    pub fn checked(&mut self, found: Option<(String, Option<String>)>, at: String) {
        self.checked_at = Some(at);
        self.error = None;
        match found {
            Some((version, notes)) => {
                self.phase = UpdatePhase::Available;
                self.release_url = Some(release_url(&version));
                self.latest_version = Some(version);
                self.notes = notes.filter(|n| !n.trim().is_empty());
            }
            None => {
                self.phase = UpdatePhase::UpToDate;
                self.latest_version = None;
                self.notes = None;
                self.release_url = None;
            }
        }
    }

    /// A check failed: what was known stays (an available update is still shown).
    pub fn check_failed(&mut self, error: String) {
        self.error = Some(error);
        self.phase = if self.latest_version.is_some() {
            UpdatePhase::Available
        } else {
            UpdatePhase::Idle
        };
    }

    /// Whether the user may start installing now.
    #[must_use]
    pub fn may_install(&self) -> bool {
        self.install == Install::InApp
            && self.latest_version.is_some()
            && matches!(self.phase, UpdatePhase::Available | UpdatePhase::Failed)
    }

    pub fn waiting(&mut self) {
        self.phase = UpdatePhase::Waiting;
        self.error = None;
        self.downloaded = 0;
        self.total = None;
    }

    pub fn progress(&mut self, chunk: u64, total: Option<u64>) {
        self.phase = UpdatePhase::Downloading;
        self.downloaded = self.downloaded.saturating_add(chunk);
        self.total = total;
    }

    pub fn installing(&mut self) {
        self.phase = UpdatePhase::Installing;
    }

    pub fn failed(&mut self, error: String) {
        self.phase = UpdatePhase::Failed;
        self.error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_modes() {
        assert_eq!(Install::detect(true, true, false), Install::Never);
        assert_eq!(Install::detect(true, false, true), Install::Never);
        assert_eq!(Install::detect(false, true, false), Install::InApp);
        assert_eq!(Install::detect(false, false, true), Install::InApp);
        // Linux without an AppImage: a .deb install.
        assert_eq!(Install::detect(false, false, false), Install::ReleasePage);
    }

    #[test]
    fn a_check_then_an_install() {
        let mut s = UpdateState::new("0.1.0".into(), Install::InApp);
        assert!(s.may_check());
        assert!(!s.may_install());
        s.checking();
        assert!(!s.may_check());
        s.checked(Some(("0.2.0".into(), Some("Fixes".into()))), "t1".into());
        assert_eq!(s.phase, UpdatePhase::Available);
        assert_eq!(
            s.release_url.as_deref(),
            Some("https://github.com/jacopofilonzi/cloudit/releases/tag/v0.2.0")
        );
        assert!(s.may_install());
        s.waiting();
        assert!(s.busy());
        assert!(!s.may_check());
        assert!(!s.may_install());
        s.progress(10, Some(100));
        s.progress(15, Some(100));
        assert_eq!((s.phase, s.downloaded), (UpdatePhase::Downloading, 25));
        s.failed("network".into());
        // Retry possible, from the start of the download.
        assert!(s.may_install());
        s.waiting();
        assert_eq!(s.downloaded, 0);
        assert_eq!(s.error, None);
        s.installing();
        assert!(s.busy());
    }

    #[test]
    fn failed_checks_keep_what_is_known() {
        let mut s = UpdateState::new("0.1.0".into(), Install::InApp);
        s.checking();
        s.check_failed("offline".into());
        assert_eq!(s.phase, UpdatePhase::Idle);
        s.checked(Some(("0.2.0".into(), Some("  ".into()))), "t".into());
        assert_eq!(s.notes, None);
        s.checking();
        s.check_failed("offline".into());
        assert_eq!(s.phase, UpdatePhase::Available);
        assert_eq!(s.latest_version.as_deref(), Some("0.2.0"));
        s.checked(None, "t2".into());
        assert_eq!(s.phase, UpdatePhase::UpToDate);
        assert_eq!(s.latest_version, None);
    }

    #[test]
    fn other_builds_never_install() {
        for install in [Install::ReleasePage, Install::Never] {
            let mut s = UpdateState::new("0.1.0".into(), install);
            s.checked(Some(("0.2.0".into(), None)), "t".into());
            assert_eq!(s.phase, UpdatePhase::Available);
            assert!(!s.may_install());
        }
    }
}
