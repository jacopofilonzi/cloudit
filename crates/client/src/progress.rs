//! Progress reporting and cancellation of long operations.
//!
//! Every operation that touches the network or the disk takes an [`OpContext`]: an
//! optional progress callback (called from engine tasks, possibly from several threads:
//! keep it cheap, e.g. forward to a channel) and a [`CancellationToken`]. Cancelling
//! stops the operation at the next step; it never leaves a partially written file in
//! place (staged temp files are removed) and the folder's base only moves once a whole
//! operation succeeded.

use std::fmt;
use std::sync::Arc;

use serde::Serialize;
pub use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};

/// What an operation is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// Reading the folder: `files` seen so far, `hashed_bytes` read to hash changed files.
    Scanning { files: u64, hashed_bytes: u64 },
    /// Asking the server which chunks it lacks.
    Checking { chunks: u64 },
    /// Uploading the missing chunks (uncompressed sizes).
    Uploading {
        done_bytes: u64,
        total_bytes: u64,
        done_chunks: u64,
        total_chunks: u64,
    },
    /// Committing the new version.
    Committing,
    /// Downloading the chunks the local files don't have.
    Downloading {
        done_bytes: u64,
        total_bytes: u64,
        done_chunks: u64,
        total_chunks: u64,
    },
    /// Writing the new files next to their targets (temp files).
    Staging { done: u64, total: u64 },
    /// Replacing and deleting local files.
    Applying { done: u64, total: u64 },
}

/// One progress report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// The linked folder's local id.
    pub folder: i64,
    #[serde(flatten)]
    pub phase: Phase,
}

/// The progress callback.
pub type ProgressFn = Arc<dyn Fn(&Progress) + Send + Sync>;

/// Progress callback and cancellation of one operation.
#[derive(Clone, Default)]
pub struct OpContext {
    progress: Option<ProgressFn>,
    cancel: CancellationToken,
}

impl fmt::Debug for OpContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpContext")
            .field("progress", &self.progress.is_some())
            .field("cancelled", &self.cancel.is_cancelled())
            .finish()
    }
}

impl OpContext {
    /// No progress, never cancelled (unless through [`OpContext::cancel_token`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// With a progress callback.
    #[must_use]
    pub fn with_progress(mut self, progress: impl Fn(&Progress) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(progress));
        self
    }

    /// With the caller's cancellation token.
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// The cancellation token (cancel it to stop the operation).
    #[must_use]
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// `Err(Cancelled)` once cancelled.
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Reports `phase` of `folder`.
    pub fn report(&self, folder: i64, phase: Phase) {
        if let Some(progress) = &self.progress {
            progress(&Progress { folder, phase });
        }
    }

    /// Runs `future` unless cancelled first.
    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        self.cancel
            .run_until_cancelled(future)
            .await
            .unwrap_or(Err(Error::Cancelled))
    }
}
