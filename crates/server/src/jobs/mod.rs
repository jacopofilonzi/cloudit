//! Background maintenance jobs, run by any instance, one instance at a time per job.
//!
//! - **Scheduling** ([`scheduler`]): each instance with `JOBS_ENABLED` (default) runs
//!   one scheduler task. Every [`scheduler::TICK`] (plus up to 5 s of random jitter) it
//!   reads the `jobs` rows; a job is **due** when an immediate run was requested
//!   (`requested_at`), it never ran, or its [`interval`] elapsed since
//!   `last_started_at` (shared by every instance, so a job runs about once per interval
//!   cluster-wide, not once per instance). A request through the admin API also wakes
//!   the local scheduler at once.
//! - **Leases**: a due job runs only after taking the `job:<name>` lease
//!   (`db::lease`, TTL [`scheduler::LEASE_TTL`], renewed every
//!   [`scheduler::RENEW_EVERY`] while running). The winner checks again that the job
//!   is still due, records the start and runs it. If renewing fails the run stops at
//!   its next check ([`RunContext::check`]) and is recorded `interrupted`.
//! - **Idempotent**: every job works in small batches, each one a transaction that
//!   re-checks its conditions; a crash or a lost lease at any point leaves consistent
//!   data and the next run continues. A run left `running` by a dead instance is marked
//!   `interrupted` when the job next starts.
//! - **Runs** are recorded in `job_runs` (outcome, counts, error) for the admin API
//!   (`GET /admin/jobs`, `POST /admin/jobs/{name}/run`); `cleanup` deletes finished runs
//!   older than [`cleanup::KEEP_JOB_RUNS`].
//!
//! The jobs: [`retention`] (every 15 min), [`gc`] (1 h), [`orphans`] (`orphan_sweep`,
//! daily), [`trash_purge`] (1 h, and on request), [`cleanup`] (1 h).

pub mod cleanup;
pub mod scheduler;
pub mod store;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cloudit_core::protocol::JobName;

use crate::state::AppState;

/// Time between two scheduled runs of `job`.
#[must_use]
pub fn interval(job: JobName) -> Duration {
    match job {
        JobName::Retention => Duration::from_secs(15 * 60),
        JobName::Gc | JobName::TrashPurge | JobName::Cleanup => Duration::from_secs(3600),
        JobName::OrphanSweep => Duration::from_secs(24 * 3600),
    }
}

/// What a run did (`job_runs.counts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts(pub BTreeMap<String, u64>);

impl Counts {
    pub fn add(&mut self, key: &str, n: u64) {
        let value = self.0.entry(key.to_owned()).or_default();
        *value = value.saturating_add(n);
    }

    #[must_use]
    pub fn get(&self, key: &str) -> u64 {
        self.0.get(key).copied().unwrap_or(0)
    }
}

/// What a running job may need besides the state: whether it still holds its lease,
/// and the GC grace period (tests shorten it).
#[derive(Debug, Clone)]
pub struct RunContext {
    lost: Arc<AtomicBool>,
    pub gc_grace: Duration,
}

impl RunContext {
    #[must_use]
    pub fn new(gc_grace: Duration) -> Self {
        Self {
            lost: Arc::new(AtomicBool::new(false)),
            gc_grace,
        }
    }

    /// Fails once the lease is lost: jobs call it between batches and stop.
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.lost.load(Ordering::Relaxed), "lease lost");
        Ok(())
    }

    fn mark_lost(&self) {
        self.lost.store(true, Ordering::Relaxed);
    }
}

/// Runs `job` once (the caller holds its lease), adding to `counts` as it goes.
pub async fn run_job(
    state: &AppState,
    job: JobName,
    ctx: &RunContext,
    counts: &mut Counts,
) -> anyhow::Result<()> {
    match job {
        JobName::Cleanup => cleanup::run(state, ctx, counts).await,
        // Implemented in the following modules.
        JobName::Retention | JobName::Gc | JobName::OrphanSweep | JobName::TrashPurge => Ok(()),
    }
}

/// Asks for an immediate run of `job` on any instance, and wakes this instance's
/// scheduler.
pub async fn request(state: &AppState, job: JobName) -> anyhow::Result<()> {
    store::request(&state.db, job).await?;
    state.job_wakeup.notify_one();
    Ok(())
}
