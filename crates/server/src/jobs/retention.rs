//! `retention` (every 15 min): deletes the snapshots each folder doesn't keep.
//!
//! - **Settings**: the folder's `retention_max_versions` / `retention_max_per_day`,
//!   else the server defaults (`settings`); 0 = unlimited. Folders in the trash, or of
//!   a trashed scope, are left alone (they come back as they were, or get purged).
//! - **Candidates**: every snapshot of the folder except the head, pinned snapshots and
//!   unresolved conflicts; those are never deleted and don't count.
//! - **Selection** ([`select`], a pure function): newest first; within each UTC day keep
//!   the `max_per_day` most recent candidates; of what remains keep the `max_versions`
//!   most recent; delete the rest.
//! - **Deleting** a snapshot ([`delete_snapshot`]) is one transaction whose first
//!   statement locks the snapshot row with a conditional `UPDATE` that re-checks
//!   eligibility (not pinned, not an unresolved conflict, not any folder's head): a pin
//!   or a restore racing with it either commits first (the snapshot is skipped) or waits
//!   and then finds it gone. Then its `snapshot_chunks`, then the row. `parent_id` and
//!   `restored_from` of other snapshots have no foreign key: they may keep naming a
//!   deleted snapshot (a dangling id, documented in the API). The chunks and manifests
//!   that are no longer referenced are left to `gc`.
//! - Each folder with deletions gets a `history_changed` event.

use std::collections::BTreeMap;

use anyhow::Context as _;
use chrono::{DateTime, NaiveDate, Utc};
use cloudit_core::protocol::SnapshotKind;
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ColumnTrait as _, Condition, DatabaseConnection, EntityTrait as _, QueryFilter as _,
    QueryOrder as _, QuerySelect as _, TransactionTrait as _,
};
use uuid::Uuid;

use super::{Counts, RunContext};
use crate::accounts::settings;
use crate::db::entities::{folders, scopes, snapshot_chunks, snapshots};
use crate::events::notify;
use crate::state::AppState;

/// Folders loaded per page.
const PAGE: u64 = 200;

/// A deletable snapshot: its id and creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// The candidates to delete (see the module docs); 0 = unlimited for either limit.
#[must_use]
pub fn select(candidates: &[Candidate], max_versions: u32, max_per_day: u32) -> Vec<Uuid> {
    let mut sorted: Vec<Candidate> = candidates.to_vec();
    // Newest first; ids (UUID v7) break ties.
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    let mut per_day: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    let mut kept: u32 = 0;
    let mut delete = Vec::new();
    for candidate in sorted {
        let day = per_day
            .entry(candidate.created_at.date_naive())
            .or_default();
        if max_per_day != 0 && *day >= max_per_day {
            delete.push(candidate.id);
            continue;
        }
        *day += 1;
        if max_versions != 0 && kept >= max_versions {
            delete.push(candidate.id);
            continue;
        }
        kept += 1;
    }
    delete
}

/// Deletes a snapshot if it is still deletable (see the module docs); `false` if it
/// was kept or is gone.
pub async fn delete_snapshot(db: &DatabaseConnection, id: Uuid) -> anyhow::Result<bool> {
    let tx = db.begin().await.context("starting a transaction")?;
    let heads = Query::select()
        .expr(Expr::val(1))
        .from(folders::Entity)
        .and_where(
            Expr::col((folders::Entity, folders::Column::HeadSnapshotId))
                .equals((snapshots::Entity, snapshots::Column::Id)),
        )
        .to_owned();
    let locked = snapshots::Entity::update_many()
        .col_expr(
            snapshots::Column::Pinned,
            Expr::col(snapshots::Column::Pinned),
        )
        .filter(snapshots::Column::Id.eq(id))
        .filter(snapshots::Column::Pinned.eq(false))
        .filter(
            Condition::any()
                .add(snapshots::Column::Kind.ne(SnapshotKind::Conflict.as_str()))
                .add(snapshots::Column::ConflictResolvedAt.is_not_null()),
        )
        .filter(Expr::exists(heads).not())
        .exec(&tx)
        .await
        .context("locking a snapshot")?
        .rows_affected
        == 1;
    if !locked {
        tx.rollback().await.context("rolling back")?;
        return Ok(false);
    }
    snapshot_chunks::Entity::delete_many()
        .filter(snapshot_chunks::Column::SnapshotId.eq(id))
        .exec(&tx)
        .await
        .context("deleting snapshot chunks")?;
    snapshots::Entity::delete_many()
        .filter(snapshots::Column::Id.eq(id))
        .exec(&tx)
        .await
        .context("deleting a snapshot")?;
    tx.commit()
        .await
        .context("committing a snapshot deletion")?;
    Ok(true)
}

fn limit(own: Option<i32>, default: u32) -> u32 {
    own.map_or(default, |v| u32::try_from(v).unwrap_or(0))
}

/// Applies retention to one folder; the number of snapshots deleted.
pub async fn apply(
    state: &AppState,
    folder: &folders::Model,
    defaults: settings::RetentionDefaults,
) -> anyhow::Result<u64> {
    let max_versions = limit(folder.retention_max_versions, defaults.max_versions);
    let max_per_day = limit(folder.retention_max_per_day, defaults.max_per_day);
    if max_versions == 0 && max_per_day == 0 {
        return Ok(0);
    }
    let rows: Vec<snapshots::Model> = snapshots::Entity::find()
        .filter(snapshots::Column::FolderId.eq(folder.id))
        .all(&state.db)
        .await
        .context("loading snapshots")?;
    let candidates: Vec<Candidate> = rows
        .iter()
        .filter(|s| Some(s.id) != folder.head_snapshot_id && !s.pinned)
        .filter(|s| !(s.kind() == SnapshotKind::Conflict && s.conflict_resolved_at.is_none()))
        .map(|s| Candidate {
            id: s.id,
            created_at: s.created_at,
        })
        .collect();
    let mut deleted = 0;
    for id in select(&candidates, max_versions, max_per_day) {
        if delete_snapshot(&state.db, id).await? {
            deleted += 1;
        }
    }
    if deleted > 0 {
        tracing::info!(folder_id = %folder.id, deleted, "retention deleted snapshots");
        notify::history_changed(state, folder.scope_id, folder.id).await;
    }
    Ok(deleted)
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    let defaults = settings::retention_defaults(&state.db).await?;
    let mut after: Option<Uuid> = None;
    loop {
        ctx.check()?;
        let live_scopes = Query::select()
            .column(scopes::Column::Id)
            .from(scopes::Entity)
            .and_where(Expr::col(scopes::Column::DeletedAt).is_null())
            .to_owned();
        let mut query = folders::Entity::find()
            .filter(folders::Column::DeletedAt.is_null())
            .filter(folders::Column::ScopeId.in_subquery(live_scopes))
            .order_by_asc(folders::Column::Id)
            .limit(PAGE);
        if let Some(after) = after {
            query = query.filter(folders::Column::Id.gt(after));
        }
        let page = query.all(&state.db).await.context("listing folders")?;
        let Some(last) = page.last() else {
            return Ok(());
        };
        after = Some(last.id);
        for folder in &page {
            counts.add("folders", 1);
            counts.add("snapshots_deleted", apply(state, folder, defaults).await?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-03-{day:02}T{hour:02}:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Candidates `(label, day, hour)`, ids in creation order.
    fn candidates(list: &[(&str, u32, u32)]) -> (Vec<Candidate>, BTreeMap<Uuid, String>) {
        let mut out = Vec::new();
        let mut names = BTreeMap::new();
        for (name, day, hour) in list {
            let id = Uuid::now_v7();
            names.insert(id, (*name).to_owned());
            out.push(Candidate {
                id,
                created_at: at(*day, *hour),
            });
        }
        (out, names)
    }

    #[test]
    fn selection_table() {
        let history = [
            ("a", 1, 8),
            ("b", 1, 12),
            ("c", 1, 18),
            ("d", 2, 9),
            ("e", 2, 23),
            ("f", 3, 0),
        ];
        // (max_versions, max_per_day, deleted)
        let cases: &[(u32, u32, &[&str])] = &[
            (0, 0, &[]),
            (100, 100, &[]),
            (3, 0, &["a", "b", "c"]),
            (0, 1, &["a", "b", "d"]),
            (0, 2, &["a"]),
            // Per day first (keeps c, e, f), then the 2 most recent of those.
            (2, 1, &["a", "b", "c", "d"]),
            (4, 2, &["a", "b"]),
            (1, 0, &["a", "b", "c", "d", "e"]),
        ];
        for (max_versions, max_per_day, expected) in cases {
            let (list, names) = candidates(&history);
            let mut deleted: Vec<String> = select(&list, *max_versions, *max_per_day)
                .iter()
                .map(|id| names[id].clone())
                .collect();
            deleted.sort();
            assert_eq!(
                deleted, *expected,
                "max_versions {max_versions}, max_per_day {max_per_day}"
            );
        }
    }

    #[test]
    fn days_are_utc_and_order_does_not_matter() {
        // 23:30 and 00:30 UTC are different days whatever the server's time zone.
        let (mut list, names) = candidates(&[("late", 1, 23), ("early", 2, 0), ("x", 2, 1)]);
        list.reverse();
        let deleted: Vec<String> = select(&list, 0, 1)
            .iter()
            .map(|id| names[id].clone())
            .collect();
        assert_eq!(deleted, ["early"]);
        assert!(select(&[], 1, 1).is_empty());
    }
}
