//! The `jobs` and `job_runs` tables (migration 000009).

use std::collections::BTreeMap;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{JobName, JobOutcome, JobRunInfo};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _, QuerySelect as _,
};
use uuid::Uuid;

use crate::db::entities::{job_runs, jobs};
use crate::time::{rfc3339, rfc3339_opt};

/// Inserts the row of every job that has none (idempotent; every instance at startup).
pub async fn ensure_rows(db: &impl ConnectionTrait) -> anyhow::Result<()> {
    jobs::Entity::insert_many(JobName::ALL.iter().map(|job| jobs::ActiveModel {
        name: Set(job.as_str().to_owned()),
        requested_at: Set(None),
        last_started_at: Set(None),
    }))
    .on_conflict(
        OnConflict::column(jobs::Column::Name)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await
    .context("inserting job rows")?;
    Ok(())
}

/// Every job row, by name.
pub async fn all(db: &impl ConnectionTrait) -> anyhow::Result<BTreeMap<JobName, jobs::Model>> {
    Ok(jobs::Entity::find()
        .all(db)
        .await
        .context("loading jobs")?
        .into_iter()
        .filter_map(|row| Some((JobName::parse(&row.name)?, row)))
        .collect())
}

pub async fn get(db: &impl ConnectionTrait, job: JobName) -> anyhow::Result<Option<jobs::Model>> {
    jobs::Entity::find_by_id(job.as_str())
        .one(db)
        .await
        .context("loading a job")
}

/// Asks for an immediate run (any instance's scheduler picks it up).
pub async fn request(db: &impl ConnectionTrait, job: JobName) -> anyhow::Result<()> {
    ensure_rows(db).await?;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::RequestedAt, Expr::value(crate::time::now()))
        .filter(jobs::Column::Name.eq(job.as_str()))
        .filter(jobs::Column::RequestedAt.is_null())
        .exec(db)
        .await
        .context("requesting a job run")?;
    Ok(())
}

/// Records the start of a run by `instance`: the job's `last_started_at`, the request
/// consumed, earlier runs left `running` (their instance died or lost the lease: we
/// hold it now) marked `interrupted`, and a new `running` run. Returns its id.
pub async fn start(
    db: &impl ConnectionTrait,
    job: JobName,
    instance: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Uuid> {
    ensure_rows(db).await?;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::LastStartedAt, Expr::value(now))
        .col_expr(
            jobs::Column::RequestedAt,
            Expr::value(None::<DateTime<Utc>>),
        )
        .filter(jobs::Column::Name.eq(job.as_str()))
        .exec(db)
        .await
        .context("recording a job start")?;
    job_runs::Entity::update_many()
        .col_expr(
            job_runs::Column::Outcome,
            Expr::value(JobOutcome::Interrupted.as_str()),
        )
        .col_expr(job_runs::Column::FinishedAt, Expr::value(now))
        .col_expr(
            job_runs::Column::Error,
            Expr::value("did not finish (instance stopped or lost its lease)"),
        )
        .filter(job_runs::Column::Job.eq(job.as_str()))
        .filter(job_runs::Column::Outcome.eq(JobOutcome::Running.as_str()))
        .exec(db)
        .await
        .context("closing interrupted runs")?;
    let id = Uuid::now_v7();
    job_runs::Entity::insert(job_runs::ActiveModel {
        id: Set(id),
        job: Set(job.as_str().to_owned()),
        instance_id: Set(instance.to_owned()),
        started_at: Set(now),
        finished_at: Set(None),
        outcome: Set(JobOutcome::Running.as_str().to_owned()),
        counts: Set("{}".to_owned()),
        error: Set(None),
    })
    .exec_without_returning(db)
    .await
    .context("recording a job run")?;
    Ok(id)
}

/// Records the end of a run.
pub async fn finish(
    db: &impl ConnectionTrait,
    run: Uuid,
    outcome: JobOutcome,
    counts: &BTreeMap<String, u64>,
    error: Option<String>,
) -> anyhow::Result<()> {
    let counts = serde_json::to_string(counts).context("encoding job counts")?;
    job_runs::Entity::update_many()
        .col_expr(job_runs::Column::Outcome, Expr::value(outcome.as_str()))
        .col_expr(
            job_runs::Column::FinishedAt,
            Expr::value(crate::time::now()),
        )
        .col_expr(job_runs::Column::Counts, Expr::value(counts))
        .col_expr(job_runs::Column::Error, Expr::value(error))
        .filter(job_runs::Column::Id.eq(run))
        .exec(db)
        .await
        .context("recording a job end")?;
    Ok(())
}

/// The latest `limit` runs of a job, newest first.
pub async fn recent_runs(
    db: &impl ConnectionTrait,
    job: JobName,
    limit: u64,
) -> anyhow::Result<Vec<job_runs::Model>> {
    job_runs::Entity::find()
        .filter(job_runs::Column::Job.eq(job.as_str()))
        .order_by_desc(job_runs::Column::Id)
        .limit(limit)
        .all(db)
        .await
        .context("listing job runs")
}

/// Deletes finished runs started before `before`; the number deleted.
pub async fn delete_runs_before(
    db: &impl ConnectionTrait,
    before: DateTime<Utc>,
) -> anyhow::Result<u64> {
    let result = job_runs::Entity::delete_many()
        .filter(job_runs::Column::StartedAt.lt(before))
        .filter(job_runs::Column::Outcome.ne(JobOutcome::Running.as_str()))
        .exec(db)
        .await
        .context("deleting old job runs")?;
    Ok(result.rows_affected)
}

/// The API view of a run.
#[must_use]
pub fn run_info(run: &job_runs::Model) -> Option<JobRunInfo> {
    Some(JobRunInfo {
        id: run.id.to_string(),
        job: JobName::parse(&run.job)?,
        instance_id: run.instance_id.clone(),
        started_at: rfc3339(run.started_at),
        finished_at: rfc3339_opt(run.finished_at),
        outcome: JobOutcome::parse(&run.outcome),
        counts: serde_json::from_str(&run.counts).unwrap_or_default(),
        error: run.error.clone(),
    })
}
