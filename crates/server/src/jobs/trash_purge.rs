//! `trash_purge` (hourly, and right after a permanent delete request): deletes for
//! good the scopes and folders in the trash for more than `trash_retention_days`, or
//! marked with `purge_requested_at`.
//!
//! - **Scope** ([`purge_scope`]): one transaction whose first statement locks the scope
//!   row with a conditional `UPDATE` re-checking it is still eligible (a concurrent
//!   restore either won, and the scope is skipped, or waits and then finds it gone).
//!   It then deletes the scope's `snapshot_chunks`, `snapshots`, `folders`,
//!   `scope_members`, `chunks`, `manifests`, `object_deletions` and the scope row (its
//!   usage goes with it). After the commit the objects under `scopes/{id}/` are listed
//!   and deleted: nobody can reference them (the namespace is per scope) and no row can
//!   appear any more (`sync::objects::lock_scope` finds no scope). A crash before the
//!   objects are gone leaves orphans for `orphan_sweep`.
//! - **Folder** ([`purge_folder`]): one transaction locking the folder row the same
//!   way, then its snapshots' `snapshot_chunks`, its `snapshots` and the folder. Its
//!   chunks and manifests become unreferenced: `gc` deletes them at its next run (an
//!   immediate `gc` run is requested), never right here, because other folders of the
//!   scope may share them and GC alone owns that decision.
//! - Recipients of the `deleted` events are computed before deleting (afterwards the
//!   memberships are gone); owners get `usage_changed`.

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{ChangeKind, JobName};
use futures::StreamExt as _;
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ColumnTrait as _, Condition, EntityTrait as _, QueryFilter as _, QuerySelect as _,
    TransactionTrait as _,
};
use uuid::Uuid;

use super::gc::delete_objects;
use super::{Counts, RunContext, store};
use crate::accounts::settings;
use crate::db::entities::{
    chunks, folders, manifests, object_deletions, scope_members, scopes, snapshot_chunks, snapshots,
};
use crate::events::notify;
use crate::state::AppState;
use crate::sync::objects::DELETE_BATCH;

fn scope_eligible(cutoff: DateTime<Utc>) -> Condition {
    Condition::all()
        .add(scopes::Column::DeletedAt.is_not_null())
        .add(
            Condition::any()
                .add(scopes::Column::DeletedAt.lt(cutoff))
                .add(scopes::Column::PurgeRequestedAt.is_not_null()),
        )
}

fn folder_eligible(cutoff: DateTime<Utc>) -> Condition {
    Condition::all()
        .add(folders::Column::DeletedAt.is_not_null())
        .add(
            Condition::any()
                .add(folders::Column::DeletedAt.lt(cutoff))
                .add(folders::Column::PurgeRequestedAt.is_not_null()),
        )
}

/// Deletes a scope and everything in it if it is still eligible; `false` otherwise.
pub async fn purge_scope(
    state: &AppState,
    scope: Uuid,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let Some(row) = scopes::Entity::find_by_id(scope)
        .one(&state.db)
        .await
        .context("loading a scope")?
    else {
        return Ok(false);
    };
    let recipients = notify::scope_recipients(&state.db, scope).await?;
    let tx = state.db.begin().await.context("starting a transaction")?;
    let locked = scopes::Entity::update_many()
        .col_expr(
            scopes::Column::DeletedAt,
            Expr::col(scopes::Column::DeletedAt),
        )
        .filter(scopes::Column::Id.eq(scope))
        .filter(scope_eligible(cutoff))
        .exec(&tx)
        .await
        .context("locking a scope for purge")?
        .rows_affected
        == 1;
    if !locked {
        tx.rollback().await.context("rolling back")?;
        return Ok(false);
    }
    snapshot_chunks::Entity::delete_many()
        .filter(snapshot_chunks::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting snapshot chunks")?;
    snapshots::Entity::delete_many()
        .filter(snapshots::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting snapshots")?;
    folders::Entity::delete_many()
        .filter(folders::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting folders")?;
    scope_members::Entity::delete_many()
        .filter(scope_members::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting members")?;
    chunks::Entity::delete_many()
        .filter(chunks::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting chunk rows")?;
    manifests::Entity::delete_many()
        .filter(manifests::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting manifest rows")?;
    object_deletions::Entity::delete_many()
        .filter(object_deletions::Column::ScopeId.eq(scope))
        .exec(&tx)
        .await
        .context("deleting tombstones")?;
    scopes::Entity::delete_many()
        .filter(scopes::Column::Id.eq(scope))
        .exec(&tx)
        .await
        .context("deleting a scope")?;
    tx.commit().await.context("committing a scope purge")?;
    tracing::info!(scope_id = %scope, "scope deleted for good");

    let mut listing = state.blobs.list(&format!("scopes/{scope}"))?;
    let mut keys = Vec::new();
    while let Some(meta) = listing.next().await {
        keys.push(meta?.key);
        if keys.len() >= DELETE_BATCH {
            delete_objects(state, &std::mem::take(&mut keys)).await?;
        }
    }
    delete_objects(state, &keys).await?;

    state.events.publish(
        recipients,
        notify::scope_changed_event(scope, ChangeKind::Deleted),
    );
    notify::usage_changed(state, row.owner_id);
    Ok(true)
}

/// Deletes a folder and its snapshots if it is still eligible; `false` otherwise.
pub async fn purge_folder(
    state: &AppState,
    folder: &folders::Model,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let tx = state.db.begin().await.context("starting a transaction")?;
    let locked = folders::Entity::update_many()
        .col_expr(
            folders::Column::DeletedAt,
            Expr::col(folders::Column::DeletedAt),
        )
        .filter(folders::Column::Id.eq(folder.id))
        .filter(folder_eligible(cutoff))
        .exec(&tx)
        .await
        .context("locking a folder for purge")?
        .rows_affected
        == 1;
    if !locked {
        tx.rollback().await.context("rolling back")?;
        return Ok(false);
    }
    let ids = Query::select()
        .column(snapshots::Column::Id)
        .from(snapshots::Entity)
        .and_where(Expr::col(snapshots::Column::FolderId).eq(folder.id))
        .to_owned();
    snapshot_chunks::Entity::delete_many()
        .filter(snapshot_chunks::Column::SnapshotId.in_subquery(ids))
        .exec(&tx)
        .await
        .context("deleting snapshot chunks")?;
    snapshots::Entity::delete_many()
        .filter(snapshots::Column::FolderId.eq(folder.id))
        .exec(&tx)
        .await
        .context("deleting snapshots")?;
    folders::Entity::delete_many()
        .filter(folders::Column::Id.eq(folder.id))
        .exec(&tx)
        .await
        .context("deleting a folder")?;
    tx.commit().await.context("committing a folder purge")?;
    tracing::info!(folder_id = %folder.id, "folder deleted for good");
    notify::folder_changed(state, folder.scope_id, folder.id, ChangeKind::Deleted).await;
    Ok(true)
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    let days = settings::trash_retention_days(&state.db).await?;
    let keep = chrono::TimeDelta::days(i64::from(days));
    let cutoff = crate::time::now() - keep;

    let scope_ids: Vec<Uuid> = scopes::Entity::find()
        .select_only()
        .column(scopes::Column::Id)
        .filter(scope_eligible(cutoff))
        .into_tuple()
        .all(&state.db)
        .await
        .context("listing scopes to purge")?;
    for scope in scope_ids {
        ctx.check()?;
        if purge_scope(state, scope, cutoff).await? {
            counts.add("scopes_deleted", 1);
        }
    }

    let list = folders::Entity::find()
        .filter(folder_eligible(cutoff))
        .all(&state.db)
        .await
        .context("listing folders to purge")?;
    let mut purged = 0;
    for folder in &list {
        ctx.check()?;
        if purge_folder(state, folder, cutoff).await? {
            purged += 1;
        }
    }
    counts.add("folders_deleted", purged);
    if purged > 0 {
        // Their chunks and manifests are garbage now: collect them soon.
        store::request(&state.db, JobName::Gc).await?;
    }
    Ok(())
}
