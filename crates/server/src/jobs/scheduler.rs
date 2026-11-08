//! The scheduler task and lease-guarded runs (rules in the [`super`] docs).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use cloudit_core::protocol::{JobName, JobOutcome};

use super::{Counts, RunContext, interval, run_job, store};
use crate::db::entities::jobs;
use crate::hooks::{self, Point};
use crate::state::AppState;

/// How often the scheduler looks for due jobs (plus jitter).
pub const TICK: Duration = Duration::from_secs(15);
/// Most random delay added to each tick.
const TICK_JITTER_MS: u64 = 5000;
/// Lease TTL of a running job; renewed every [`RENEW_EVERY`].
pub const LEASE_TTL: Duration = Duration::from_secs(5 * 60);
pub const RENEW_EVERY: Duration = Duration::from_secs(60);

fn lease_name(job: JobName) -> String {
    format!("job:{}", job.as_str())
}

/// Whether `job` should run now.
#[must_use]
pub fn is_due(job: JobName, row: Option<&jobs::Model>, now: DateTime<Utc>) -> bool {
    let Some(row) = row else {
        return true;
    };
    if row.requested_at.is_some() {
        return true;
    }
    match row.last_started_at {
        None => true,
        Some(last) => crate::time::add(last, interval(job)).is_ok_and(|next| now >= next),
    }
}

/// The result of one lease-guarded attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    /// Another instance holds the lease (or the job was not due any more).
    Skipped,
    /// It ran: the run id, outcome and counts.
    Ran {
        run: uuid::Uuid,
        outcome: JobOutcome,
        counts: Counts,
    },
}

/// Runs `job` if this instance gets its lease and (unless `force`) it is still due.
pub async fn run_leased(
    state: &AppState,
    job: JobName,
    force: bool,
    gc_grace: Duration,
) -> anyhow::Result<Attempt> {
    let lease = lease_name(job);
    if !state.leases.try_acquire(&lease, LEASE_TTL).await? {
        return Ok(Attempt::Skipped);
    }
    let result = run_holding_lease(state, job, force, gc_grace, &lease).await;
    if let Err(err) = state.leases.release(&lease).await {
        tracing::warn!(error = %format!("{err:#}"), job = job.as_str(), "releasing a job lease");
    }
    result
}

async fn run_holding_lease(
    state: &AppState,
    job: JobName,
    force: bool,
    gc_grace: Duration,
    lease: &str,
) -> anyhow::Result<Attempt> {
    let now = crate::time::now();
    if !force && !is_due(job, store::get(&state.db, job).await?.as_ref(), now) {
        return Ok(Attempt::Skipped);
    }
    let run = store::start(&state.db, job, state.leases.holder(), now).await?;
    hooks::reached(
        Point::JobRunning,
        &format!("{}:{}", state.leases.holder(), job.as_str()),
    )
    .await;
    tracing::info!(job = job.as_str(), %run, "job started");

    let ctx = RunContext::new(gc_grace);
    let renewer = {
        let state = state.clone();
        let ctx = ctx.clone();
        let lease = lease.to_owned();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(RENEW_EVERY);
            every.tick().await;
            loop {
                every.tick().await;
                match state.leases.renew(&lease, LEASE_TTL).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        ctx.mark_lost();
                        return;
                    }
                }
            }
        })
    };
    let mut counts = Counts::default();
    let result = run_job(state, job, &ctx, &mut counts).await;
    renewer.abort();
    let (outcome, error) = match &result {
        Ok(()) => (JobOutcome::Succeeded, None),
        Err(err) if ctx.check().is_err() => (JobOutcome::Interrupted, Some(format!("{err:#}"))),
        Err(err) => (JobOutcome::Failed, Some(format!("{err:#}"))),
    };
    match &error {
        None => tracing::info!(job = job.as_str(), %run, counts = ?counts.0, "job finished"),
        Some(error) => {
            tracing::warn!(job = job.as_str(), %run, counts = ?counts.0, error, "job failed")
        }
    }
    let error = error.map(|e| e.chars().take(1000).collect());
    store::finish(&state.db, run, outcome, &counts.0, error).await?;
    Ok(Attempt::Ran {
        run,
        outcome,
        counts,
    })
}

/// Starts this instance's scheduler (`main`, when `JOBS_ENABLED`).
pub fn start(state: AppState) -> tokio::task::JoinHandle<()> {
    start_with(state, TICK)
}

/// [`start`] with another tick (tests).
pub fn start_with(state: AppState, tick: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = store::ensure_rows(&state.db).await {
            tracing::warn!(error = %format!("{err:#}"), "preparing job rows");
        }
        let running: Arc<Mutex<HashSet<JobName>>> = Arc::default();
        tracing::info!("job scheduler started");
        loop {
            if let Err(err) = tick_once(&state, &running).await {
                tracing::warn!(error = %format!("{err:#}"), "job scheduler tick");
            }
            let jitter = Duration::from_millis(rand_below(TICK_JITTER_MS.min(tick_ms(tick))));
            tokio::select! {
                () = tokio::time::sleep(tick + jitter) => {}
                () = state.job_wakeup.notified() => {}
            }
        }
    })
}

fn tick_ms(tick: Duration) -> u64 {
    u64::try_from(tick.as_millis()).unwrap_or(u64::MAX)
}

/// A random number below `n` (0 if `n` is 0).
fn rand_below(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return 0;
    }
    u64::from_le_bytes(bytes) % n
}

/// Starts every due job not already running on this instance.
async fn tick_once(state: &AppState, running: &Arc<Mutex<HashSet<JobName>>>) -> anyhow::Result<()> {
    let rows = store::all(&state.db).await?;
    let now = crate::time::now();
    for job in JobName::ALL {
        if !is_due(job, rows.get(&job), now) {
            continue;
        }
        {
            let mut running = running
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduler state poisoned"))?;
            if !running.insert(job) {
                continue;
            }
        }
        let state = state.clone();
        let running = running.clone();
        tokio::spawn(async move {
            let grace = state.config.gc_grace;
            if let Err(err) = run_leased(&state, job, false, grace).await {
                tracing::warn!(error = %format!("{err:#}"), job = job.as_str(), "job run");
            }
            if let Ok(mut running) = running.lock() {
                running.remove(&job);
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(requested: bool, last: Option<DateTime<Utc>>) -> jobs::Model {
        jobs::Model {
            name: "gc".to_owned(),
            requested_at: requested.then(crate::time::now),
            last_started_at: last,
        }
    }

    #[test]
    fn due_rules() {
        let now = crate::time::now();
        let hour = chrono::TimeDelta::hours(1);
        assert!(is_due(JobName::Gc, None, now));
        assert!(is_due(JobName::Gc, Some(&row(false, None)), now));
        assert!(!is_due(JobName::Gc, Some(&row(false, Some(now))), now));
        assert!(is_due(JobName::Gc, Some(&row(true, Some(now))), now));
        assert!(is_due(
            JobName::Gc,
            Some(&row(false, Some(now - hour))),
            now
        ));
        assert!(!is_due(
            JobName::OrphanSweep,
            Some(&row(false, Some(now - hour))),
            now
        ));
    }
}
