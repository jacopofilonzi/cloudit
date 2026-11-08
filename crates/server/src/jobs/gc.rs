//! `gc` (hourly): deletes the `chunks` and `manifests` rows no snapshot references any
//! more and that are older than `GC_GRACE`, then their objects (protocol and race
//! argument: `sync::objects`).
//!
//! Per scope:
//!
//! 1. **Rows**, in batches of [`ROW_BATCH`]: select candidates (unreferenced, created
//!    before `now - GC_GRACE`), then one transaction: the scope lock, `DELETE … WHERE`
//!    the same conditions again `RETURNING` what was deleted, a tombstone per deleted
//!    row (`object_deletions`), `scopes.stored_bytes -= ` their sizes. Only rows
//!    actually deleted are counted, so usage stays equal to `usage::recompute`. A
//!    concurrent commit that references a row makes the delete fail on the foreign key
//!    (PostgreSQL): the batch rolls back and the scope is retried at the next run.
//! 2. **Objects**: [`delete_tombstoned`], batches of `objects::DELETE_BATCH` under the
//!    scope lock: an object is deleted only if its row is still absent (a new upload
//!    may have inserted it again); the tombstones go in the same transaction.
//!
//! The grace period covers a client told by `check` that a chunk exists, and commits
//! racing the collector: if a row goes anyway, the commit answers `chunk_missing` and
//! the client uploads again. Owners whose usage went down get `usage_changed`.

use std::collections::HashSet;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict, Query, SimpleExpr};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, DatabaseConnection, EntityTrait as _, QueryFilter as _,
    QueryOrder as _, QuerySelect as _, TransactionTrait as _,
};
use uuid::Uuid;

use super::{Counts, RunContext};
use crate::db::entities::{
    chunks, manifests, object_deletions, scopes, snapshot_chunks, snapshots,
};
use crate::events::notify;
use crate::hooks::{self, Point};
use crate::state::AppState;
use crate::sync::objects::{self, DELETE_BATCH, DELETE_CONCURRENCY, Kind};

/// Rows deleted per transaction.
pub const ROW_BATCH: u64 = 500;
/// Scopes loaded per page.
const SCOPE_PAGE: u64 = 200;

/// `NOT EXISTS (a snapshot_chunks row for this chunk)`.
fn chunk_unreferenced() -> SimpleExpr {
    let refs = Query::select()
        .expr(Expr::val(1))
        .from(snapshot_chunks::Entity)
        .and_where(
            Expr::col((snapshot_chunks::Entity, snapshot_chunks::Column::ScopeId))
                .equals((chunks::Entity, chunks::Column::ScopeId)),
        )
        .and_where(
            Expr::col((snapshot_chunks::Entity, snapshot_chunks::Column::Hash))
                .equals((chunks::Entity, chunks::Column::Hash)),
        )
        .to_owned();
    Expr::exists(refs).not()
}

/// `NOT EXISTS (a snapshot with this manifest)`.
fn manifest_unreferenced() -> SimpleExpr {
    let refs = Query::select()
        .expr(Expr::val(1))
        .from(snapshots::Entity)
        .and_where(
            Expr::col((snapshots::Entity, snapshots::Column::ScopeId))
                .equals((manifests::Entity, manifests::Column::ScopeId)),
        )
        .and_where(
            Expr::col((snapshots::Entity, snapshots::Column::ManifestHash))
                .equals((manifests::Entity, manifests::Column::Hash)),
        )
        .to_owned();
    Expr::exists(refs).not()
}

/// Candidate hashes of `kind` in `scope`.
async fn candidates(
    db: &DatabaseConnection,
    scope: Uuid,
    kind: Kind,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<String> = match kind {
        Kind::Chunk => {
            chunks::Entity::find()
                .select_only()
                .column(chunks::Column::Hash)
                .filter(chunks::Column::ScopeId.eq(scope))
                .filter(chunks::Column::CreatedAt.lt(cutoff))
                .filter(chunk_unreferenced())
                .limit(ROW_BATCH)
                .into_tuple()
                .all(db)
                .await
        }
        Kind::Manifest => {
            manifests::Entity::find()
                .select_only()
                .column(manifests::Column::Hash)
                .filter(manifests::Column::ScopeId.eq(scope))
                .filter(manifests::Column::CreatedAt.lt(cutoff))
                .filter(manifest_unreferenced())
                .limit(ROW_BATCH)
                .into_tuple()
                .all(db)
                .await
        }
    }
    .context("selecting unreferenced rows")?;
    Ok(rows)
}

/// Deletes the still-deletable rows of `batch` with their tombstones and usage, in one
/// transaction; the `(hash, stored_size)` deleted. `None` if the scope is gone.
async fn delete_rows(
    db: &DatabaseConnection,
    scope: Uuid,
    kind: Kind,
    batch: Vec<String>,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<Option<Vec<(String, i64)>>> {
    let tx = db.begin().await.context("starting a transaction")?;
    if !objects::lock_scope(&tx, scope).await? {
        tx.rollback().await.context("rolling back")?;
        return Ok(None);
    }
    let deleted: Vec<(String, i64)> = match kind {
        Kind::Chunk => chunks::Entity::delete_many()
            .filter(chunks::Column::ScopeId.eq(scope))
            .filter(chunks::Column::Hash.is_in(batch))
            .filter(chunks::Column::CreatedAt.lt(cutoff))
            .filter(chunk_unreferenced())
            .exec_with_returning(&tx)
            .await
            .context("deleting chunk rows")?
            .into_iter()
            .map(|row| (row.hash, row.stored_size))
            .collect(),
        Kind::Manifest => manifests::Entity::delete_many()
            .filter(manifests::Column::ScopeId.eq(scope))
            .filter(manifests::Column::Hash.is_in(batch))
            .filter(manifests::Column::CreatedAt.lt(cutoff))
            .filter(manifest_unreferenced())
            .exec_with_returning(&tx)
            .await
            .context("deleting manifest rows")?
            .into_iter()
            .map(|row| (row.hash, row.stored_size))
            .collect(),
    };
    if !deleted.is_empty() {
        let now = crate::time::now();
        object_deletions::Entity::insert_many(deleted.iter().map(|(hash, _)| {
            object_deletions::ActiveModel {
                scope_id: Set(scope),
                kind: Set(kind.as_str().to_owned()),
                hash: Set(hash.clone()),
                created_at: Set(now),
            }
        }))
        .on_conflict(
            OnConflict::columns([
                object_deletions::Column::ScopeId,
                object_deletions::Column::Kind,
                object_deletions::Column::Hash,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&tx)
        .await
        .context("recording object deletions")?;
        let freed: i64 = deleted.iter().map(|(_, size)| *size).sum();
        scopes::Entity::update_many()
            .col_expr(
                scopes::Column::StoredBytes,
                Expr::col(scopes::Column::StoredBytes).sub(freed),
            )
            .filter(scopes::Column::Id.eq(scope))
            .exec(&tx)
            .await
            .context("uncharging the scope")?;
    }
    tx.commit().await.context("committing a row deletion")?;
    Ok(Some(deleted))
}

/// Deletes the objects of the scope's tombstones whose row is still absent, and the
/// tombstones; the number of objects deleted.
pub async fn delete_tombstoned(state: &AppState, scope: Uuid) -> anyhow::Result<u64> {
    let mut deleted = 0;
    loop {
        let batch = object_deletions::Entity::find()
            .filter(object_deletions::Column::ScopeId.eq(scope))
            .order_by_asc(object_deletions::Column::Hash)
            .limit(DELETE_BATCH as u64)
            .all(&state.db)
            .await
            .context("loading object deletions")?;
        if batch.is_empty() {
            return Ok(deleted);
        }
        let tx = state.db.begin().await.context("starting a transaction")?;
        // Without a scope row no row can appear: delete without the lock.
        let locked = objects::lock_scope(&tx, scope).await?;
        let mut keys = Vec::new();
        for tomb in &batch {
            let Some(kind) = Kind::parse(&tomb.kind) else {
                continue;
            };
            if locked && objects::row_exists(&tx, scope, kind, &tomb.hash).await? {
                continue;
            }
            keys.push(kind.key(scope, &tomb.hash));
        }
        delete_objects(state, &keys).await?;
        deleted += keys.len() as u64;
        for tomb in &batch {
            object_deletions::Entity::delete_many()
                .filter(object_deletions::Column::ScopeId.eq(scope))
                .filter(object_deletions::Column::Kind.eq(tomb.kind.as_str()))
                .filter(object_deletions::Column::Hash.eq(tomb.hash.as_str()))
                .exec(&tx)
                .await
                .context("deleting a tombstone")?;
        }
        tx.commit().await.context("committing object deletions")?;
    }
}

/// Deletes these objects, [`DELETE_CONCURRENCY`] at a time.
pub async fn delete_objects(state: &AppState, keys: &[String]) -> anyhow::Result<()> {
    let deletes = keys.iter().cloned().map(|key| {
        let blobs = state.blobs.clone();
        async move { blobs.delete(&key).await }
    });
    let results: Vec<anyhow::Result<()>> = futures::stream::iter(deletes)
        .buffer_unordered(DELETE_CONCURRENCY)
        .collect()
        .await;
    results.into_iter().collect()
}

/// Collects one scope; the bytes freed.
async fn collect_scope(
    state: &AppState,
    ctx: &RunContext,
    scope: Uuid,
    cutoff: DateTime<Utc>,
    counts: &mut Counts,
) -> anyhow::Result<u64> {
    let mut freed: u64 = 0;
    'kinds: for kind in [Kind::Chunk, Kind::Manifest] {
        loop {
            ctx.check()?;
            let batch = candidates(&state.db, scope, kind, cutoff).await?;
            if batch.is_empty() {
                break;
            }
            match delete_rows(&state.db, scope, kind, batch, cutoff).await {
                Ok(None) => break 'kinds,
                Ok(Some(deleted)) => {
                    if deleted.is_empty() {
                        break;
                    }
                    let bytes: i64 = deleted.iter().map(|(_, s)| *s).sum();
                    freed = freed.saturating_add(u64::try_from(bytes).unwrap_or(0));
                    let key = match kind {
                        Kind::Chunk => "chunks_deleted",
                        Kind::Manifest => "manifests_deleted",
                    };
                    counts.add(key, deleted.len() as u64);
                }
                Err(err) => {
                    // Typically a commit referencing a row meanwhile (foreign key):
                    // nothing was changed; the next run retries.
                    tracing::warn!(scope_id = %scope, error = %format!("{err:#}"), "gc batch skipped");
                    counts.add("batches_skipped", 1);
                    break 'kinds;
                }
            }
        }
    }
    hooks::reached(Point::GcRowsDeleted, &scope.to_string()).await;
    counts.add("objects_deleted", delete_tombstoned(state, scope).await?);
    counts.add("bytes_freed", freed);
    Ok(freed)
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    let grace = chrono::TimeDelta::from_std(ctx.gc_grace).context("GC grace")?;
    let cutoff = crate::time::now() - grace;
    let mut owners: HashSet<Uuid> = HashSet::new();
    let mut after: Option<Uuid> = None;
    loop {
        let mut query = scopes::Entity::find()
            .select_only()
            .column(scopes::Column::Id)
            .column(scopes::Column::OwnerId)
            .order_by_asc(scopes::Column::Id)
            .limit(SCOPE_PAGE);
        if let Some(after) = after {
            query = query.filter(scopes::Column::Id.gt(after));
        }
        let page: Vec<(Uuid, Uuid)> = query
            .into_tuple()
            .all(&state.db)
            .await
            .context("listing scopes")?;
        let Some((last, _)) = page.last() else {
            break;
        };
        after = Some(*last);
        for (scope, owner) in page {
            if collect_scope(state, ctx, scope, cutoff, counts).await? > 0 {
                owners.insert(owner);
            }
        }
    }
    for owner in owners {
        notify::usage_changed(state, owner);
    }
    Ok(())
}
