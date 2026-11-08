//! `cleanup` (hourly): deletes rows that can never be used again.
//!
//! - expired `sessions`, `device_auth_codes` and `oauth_flows`;
//! - expired `pending_signups` no flow references any more;
//! - `password_resets` used or expired;
//! - `invites` never used that expired (or were revoked) more than
//!   [`KEEP_EXPIRED_INVITES`] ago, so the admin list still shows recent ones. **Used
//!   invites are kept**: they document who invited whom (`created_by` → `used_by`);
//! - finished `job_runs` older than [`KEEP_JOB_RUNS`].
//!
//! Devices are never deleted (snapshots name the device that made them); revoked and
//! expired ones stay listed for their owner. Each delete is one statement.

use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{ColumnTrait as _, Condition, EntityTrait as _, QueryFilter as _};

use super::{Counts, RunContext, store};
use crate::db::entities::{
    device_auth_codes, invites, oauth_flows, password_resets, pending_signups, sessions,
};
use crate::state::AppState;

/// Unused invites stay this long after they expire.
pub const KEEP_EXPIRED_INVITES: Duration = Duration::from_secs(30 * 86_400);
/// Finished job runs are kept this long.
pub const KEEP_JOB_RUNS: Duration = Duration::from_secs(30 * 86_400);

fn before(
    now: chrono::DateTime<chrono::Utc>,
    d: Duration,
) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    let d = chrono::TimeDelta::from_std(d).context("duration")?;
    now.checked_sub_signed(d).context("timestamp underflow")
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    let db = &state.db;
    let now = crate::time::now();

    let n = sessions::Entity::delete_many()
        .filter(sessions::Column::ExpiresAt.lt(now))
        .exec(db)
        .await
        .context("deleting expired sessions")?
        .rows_affected;
    counts.add("sessions", n);
    ctx.check()?;

    let n = device_auth_codes::Entity::delete_many()
        .filter(device_auth_codes::Column::ExpiresAt.lt(now))
        .exec(db)
        .await
        .context("deleting expired device codes")?
        .rows_affected;
    counts.add("device_auth_codes", n);

    let n = oauth_flows::Entity::delete_many()
        .filter(oauth_flows::Column::ExpiresAt.lt(now))
        .exec(db)
        .await
        .context("deleting expired sign-in flows")?
        .rows_affected;
    counts.add("oauth_flows", n);

    let referenced = Query::select()
        .expr(Expr::val(1))
        .from(oauth_flows::Entity)
        .and_where(
            Expr::col((oauth_flows::Entity, oauth_flows::Column::PendingSignupId))
                .equals((pending_signups::Entity, pending_signups::Column::Id)),
        )
        .to_owned();
    let n = pending_signups::Entity::delete_many()
        .filter(pending_signups::Column::ExpiresAt.lt(now))
        .filter(Expr::exists(referenced).not())
        .exec(db)
        .await
        .context("deleting expired pending sign-ups")?
        .rows_affected;
    counts.add("pending_signups", n);
    ctx.check()?;

    let n = password_resets::Entity::delete_many()
        .filter(
            Condition::any()
                .add(password_resets::Column::ExpiresAt.lt(now))
                .add(password_resets::Column::UsedAt.is_not_null()),
        )
        .exec(db)
        .await
        .context("deleting used or expired password resets")?
        .rows_affected;
    counts.add("password_resets", n);

    let invite_cutoff = before(now, KEEP_EXPIRED_INVITES)?;
    let n = invites::Entity::delete_many()
        .filter(invites::Column::UsedAt.is_null())
        .filter(
            Condition::any()
                .add(invites::Column::ExpiresAt.lt(invite_cutoff))
                .add(invites::Column::RevokedAt.lt(invite_cutoff)),
        )
        .exec(db)
        .await
        .context("deleting old unused invites")?
        .rows_affected;
    counts.add("invites", n);

    let n = store::delete_runs_before(db, before(now, KEEP_JOB_RUNS)?).await?;
    counts.add("job_runs", n);
    Ok(())
}
