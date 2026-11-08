//! `/api/v1/admin/jobs*` (admin session): the background jobs and their latest runs,
//! and immediate runs on request.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{JobInfo, JobName};

use crate::auth::extractors::AdminUser;
use crate::db::entities::jobs as job_entity;
use crate::errors::ApiError;
use crate::jobs::{self, store};
use crate::state::AppState;
use crate::time::rfc3339_opt;

/// Runs listed per job.
const RECENT_RUNS: u64 = 10;

async fn info(
    state: &AppState,
    job: JobName,
    row: Option<&job_entity::Model>,
) -> Result<JobInfo, ApiError> {
    let runs = store::recent_runs(&state.db, job, RECENT_RUNS).await?;
    Ok(JobInfo {
        name: job,
        interval_secs: jobs::interval(job).as_secs(),
        requested_at: rfc3339_opt(row.and_then(|r| r.requested_at)),
        last_started_at: rfc3339_opt(row.and_then(|r| r.last_started_at)),
        recent_runs: runs.iter().filter_map(store::run_info).collect(),
    })
}

/// `GET /admin/jobs`: every job, in a fixed order.
pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<JobInfo>>, ApiError> {
    let rows = store::all(&state.db).await?;
    let mut out = Vec::with_capacity(JobName::ALL.len());
    for job in JobName::ALL {
        out.push(info(&state, job, rows.get(&job)).await?);
    }
    Ok(Json(out))
}

/// `POST /admin/jobs/{name}/run` → 202: an instance with jobs enabled runs it shortly
/// (this one at once, if enabled here).
pub async fn run(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<JobInfo>), ApiError> {
    let job = JobName::parse(&name).ok_or(ApiError::NotFound)?;
    jobs::request(&state, job).await?;
    tracing::info!(admin_id = %me.user.id, job = job.as_str(), "job run requested");
    let row = store::get(&state.db, job).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(info(&state, job, row.as_ref()).await?),
    ))
}
