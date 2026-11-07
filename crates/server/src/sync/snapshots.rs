//! The `snapshots` and `snapshot_chunks` tables: commits (with the conditional head
//! update), conflicts, folder summaries.
//!
//! A commit is one transaction: insert the snapshot (the first statement is a write,
//! see the `db` conventions), insert its distinct chunk hashes in batches, then, for a
//! `normal` snapshot, `UPDATE folders SET head_snapshot_id = new WHERE id = ? AND
//! deleted_at IS NULL AND head_snapshot_id = parent` (`IS NULL` without a parent). No
//! row updated → rollback and `conflict` with the current head, so of two clients
//! committing on the same parent exactly one wins, whatever the timing or instance.
//! `conflict` snapshots never touch the head.

use std::collections::HashMap;

use anyhow::Context as _;
use cloudit_core::hash::{ChunkHash, ManifestHash};
use cloudit_core::protocol::SnapshotKind;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, DatabaseConnection, EntityTrait as _,
    QueryFilter as _, QuerySelect as _, TransactionTrait as _,
};
use uuid::Uuid;

use crate::db::entities::{folders, snapshot_chunks, snapshots};
use crate::errors::ApiError;

/// Rows per multi-row INSERT of `snapshot_chunks` (3 values each: far below the bind
/// limits of SQLite and PostgreSQL).
const INSERT_BATCH: usize = 1000;

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<Option<snapshots::Model>> {
    snapshots::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading a snapshot")
}

/// The manifest hash of a snapshot row.
pub fn manifest_hash(snapshot: &snapshots::Model) -> anyhow::Result<ManifestHash> {
    snapshot
        .manifest_hash
        .parse()
        .with_context(|| format!("stored manifest hash of snapshot {}", snapshot.id))
}

/// A snapshot to insert.
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    pub folder: Uuid,
    pub scope: Uuid,
    pub parent: Option<Uuid>,
    pub kind: SnapshotKind,
    pub manifest_hash: ManifestHash,
    pub file_count: u64,
    pub logical_size: u64,
    pub author: Uuid,
    pub device: Option<Uuid>,
    pub restored_from: Option<Uuid>,
}

fn model(new: &NewSnapshot) -> anyhow::Result<snapshots::ActiveModel> {
    Ok(snapshots::ActiveModel {
        id: Set(Uuid::now_v7()),
        folder_id: Set(new.folder),
        scope_id: Set(new.scope),
        parent_id: Set(new.parent),
        kind: Set(new.kind.as_str().to_owned()),
        manifest_hash: Set(new.manifest_hash.to_string()),
        file_count: Set(i64::try_from(new.file_count).context("file count")?),
        logical_size: Set(i64::try_from(new.logical_size).context("logical size")?),
        author_id: Set(new.author),
        device_id: Set(new.device),
        created_at: Set(crate::time::now()),
        pinned: Set(false),
        label: Set(None),
        restored_from: Set(new.restored_from),
        conflict_resolved_at: Set(None),
    })
}

/// Moves the folder's head from `from` to `to`; `false` if the head isn't `from` any
/// more (or the folder went to the trash).
async fn move_head(
    db: &impl ConnectionTrait,
    folder: Uuid,
    from: Option<Uuid>,
    to: Uuid,
) -> anyhow::Result<bool> {
    let update = folders::Entity::update_many()
        .col_expr(folders::Column::HeadSnapshotId, Expr::value(Some(to)))
        .filter(folders::Column::Id.eq(folder))
        .filter(folders::Column::DeletedAt.is_null());
    let update = match from {
        Some(parent) => update.filter(folders::Column::HeadSnapshotId.eq(parent)),
        None => update.filter(folders::Column::HeadSnapshotId.is_null()),
    };
    let result = update.exec(db).await.context("moving the folder head")?;
    Ok(result.rows_affected == 1)
}

/// The error for a head that moved: `conflict` with the current head, or `not_found`
/// if the folder went to the trash meanwhile.
async fn head_moved(db: &impl ConnectionTrait, folder: Uuid) -> ApiError {
    match crate::scopes::folders::find(db, folder).await {
        Ok(Some(f)) if f.deleted_at.is_none() => ApiError::Conflict {
            head: f.head_snapshot_id,
        },
        Ok(_) => ApiError::NotFound,
        Err(err) => err.into(),
    }
}

/// Inserts a snapshot with its distinct chunk hashes; a `normal` one also becomes the
/// head if the head still is its parent, else `conflict` (nothing is kept).
pub async fn commit(
    db: &DatabaseConnection,
    new: &NewSnapshot,
    chunks: &[ChunkHash],
) -> Result<snapshots::Model, ApiError> {
    let tx = db.begin().await.context("starting a transaction")?;
    let snapshot = snapshots::Entity::insert(model(new)?)
        .exec_with_returning(&tx)
        .await
        .context("inserting a snapshot")?;
    for batch in chunks.chunks(INSERT_BATCH) {
        snapshot_chunks::Entity::insert_many(batch.iter().map(|hash| {
            snapshot_chunks::ActiveModel {
                snapshot_id: Set(snapshot.id),
                scope_id: Set(new.scope),
                hash: Set(hash.to_string()),
            }
        }))
        .exec_without_returning(&tx)
        .await
        .context("inserting snapshot chunks")?;
    }
    if new.kind != SnapshotKind::Conflict
        && !move_head(&tx, new.folder, new.parent, snapshot.id).await?
    {
        tx.rollback().await.context("rolling back")?;
        return Err(head_moved(db, new.folder).await);
    }
    tx.commit().await.context("committing a snapshot")?;
    Ok(snapshot)
}

/// Marks a conflict snapshot resolved (idempotent); `false` if it isn't a conflict.
pub async fn resolve(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<bool> {
    snapshots::Entity::update_many()
        .col_expr(
            snapshots::Column::ConflictResolvedAt,
            Expr::value(Some(crate::time::now())),
        )
        .filter(snapshots::Column::Id.eq(id))
        .filter(snapshots::Column::Kind.eq(SnapshotKind::Conflict.as_str()))
        .filter(snapshots::Column::ConflictResolvedAt.is_null())
        .exec(db)
        .await
        .context("resolving a conflict")?;
    Ok(find(db, id)
        .await?
        .is_some_and(|s| s.kind() == SnapshotKind::Conflict))
}

/// Sync state of a folder: its head snapshot and open conflicts.
#[derive(Debug, Clone, Default)]
pub struct FolderSync {
    pub head: Option<snapshots::Model>,
    pub unresolved_conflicts: u64,
}

/// [`FolderSync`] of each folder (absent: no head, no conflicts).
pub async fn folder_sync(
    db: &impl ConnectionTrait,
    list: &[&folders::Model],
) -> anyhow::Result<HashMap<Uuid, FolderSync>> {
    let mut out: HashMap<Uuid, FolderSync> = HashMap::new();
    if list.is_empty() {
        return Ok(out);
    }
    let heads: Vec<Uuid> = list.iter().filter_map(|f| f.head_snapshot_id).collect();
    if !heads.is_empty() {
        for head in snapshots::Entity::find()
            .filter(snapshots::Column::Id.is_in(heads))
            .all(db)
            .await
            .context("loading folder heads")?
        {
            let folder = head.folder_id;
            out.entry(folder).or_default().head = Some(head);
        }
    }
    let counts: Vec<(Uuid, i64)> = snapshots::Entity::find()
        .select_only()
        .column(snapshots::Column::FolderId)
        .column_as(snapshots::Column::Id.count(), "count")
        .filter(snapshots::Column::FolderId.is_in(list.iter().map(|f| f.id)))
        .filter(snapshots::Column::Kind.eq(SnapshotKind::Conflict.as_str()))
        .filter(snapshots::Column::ConflictResolvedAt.is_null())
        .group_by(snapshots::Column::FolderId)
        .into_tuple()
        .all(db)
        .await
        .context("counting open conflicts")?;
    for (folder, count) in counts {
        out.entry(folder).or_default().unresolved_conflicts = u64::try_from(count).unwrap_or(0);
    }
    Ok(out)
}
