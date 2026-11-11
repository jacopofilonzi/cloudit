//! Storage usage and quotas.
//!
//! - **Counter**: `scopes.stored_bytes` = the `stored_size` of the scope's `chunks` and
//!   `manifests` rows. [`insert_charged`] inserts a row and increments the counter in
//!   one transaction, only when the row was actually inserted; the garbage collector
//!   (`jobs::gc`) decrements it in the transaction that deletes rows. [`recompute`]
//!   rebuilds the value from the rows (consistency checks).
//! - **Usage** of a user = the sum over the scopes it **owns**, trashed ones included:
//!   uploads by the members of a shared scope count against its owner.
//! - **Quota** = none if `users.quota_unlimited`, else `users.quota_bytes`, else the
//!   `default_quota_bytes` setting, else unlimited ([`effective_quota`]). Exceeding it
//!   gives `quota_exceeded`.
//!
//! Enforcement is exact: with a quota, [`insert_charged`] (after the scope lock of
//! `sync::objects`, always taken first) writes the owner's `users` row (a lock: on
//! PostgreSQL a row lock, on SQLite the write lock), so the charges of one owner are
//! serialized, then inserts, increments and re-reads the owner's total, rolling back
//! if it is over. The pre-checks ([`ensure_room`], before storing an object or handing
//! out upload URLs) are estimates that fail early.

use anyhow::Context as _;
use cloudit_core::hash::{ChunkHash, ManifestHash};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QuerySelect as _, TransactionTrait as _,
};
use uuid::Uuid;

use crate::accounts::{settings, users};
use crate::db::entities::{chunks, manifests, scopes, users as user_entity};
use crate::errors::ApiError;
use crate::state::AppState;
use crate::sync::objects::{self, Kind, Stored};

/// A row to insert with [`insert_charged`].
#[derive(Debug, Clone, Copy)]
pub enum NewObject<'a> {
    Chunk { hash: &'a ChunkHash, size: u32 },
    Manifest { hash: &'a ManifestHash },
}

fn to_i64(value: u64) -> anyhow::Result<i64> {
    i64::try_from(value).context("size out of range")
}

/// A stored counter as bytes (never negative).
#[must_use]
pub fn bytes(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// The effective quota of `user` given the server default: none if the account is
/// unlimited, else its own, else the default; `None` = unlimited.
#[must_use]
pub fn quota_with_default(user: &user_entity::Model, default: Option<u64>) -> Option<u64> {
    if user.quota_unlimited {
        return None;
    }
    user.quota_bytes.map(bytes).or(default)
}

/// The effective quota of `user` ([`quota_with_default`] with the server default).
pub async fn effective_quota(
    db: &impl ConnectionTrait,
    user: &user_entity::Model,
) -> anyhow::Result<Option<u64>> {
    if user.quota_unlimited {
        return Ok(None);
    }
    match user.quota_bytes {
        Some(own) => Ok(Some(bytes(own))),
        None => settings::default_quota_bytes(db).await,
    }
}

/// The effective quota of the owner of `scope`.
pub async fn owner_quota(
    db: &impl ConnectionTrait,
    scope: &scopes::Model,
) -> anyhow::Result<Option<u64>> {
    let owner = users::find(db, scope.owner_id)
        .await?
        .context("scope owner not found")?;
    effective_quota(db, &owner).await
}

/// Bytes used by `owner`: the sum over the scopes it owns (trashed included).
pub async fn used_by(db: &impl ConnectionTrait, owner: Uuid) -> anyhow::Result<u64> {
    let values: Vec<i64> = scopes::Entity::find()
        .select_only()
        .column(scopes::Column::StoredBytes)
        .filter(scopes::Column::OwnerId.eq(owner))
        .into_tuple()
        .all(db)
        .await
        .context("reading storage usage")?;
    Ok(values.into_iter().map(bytes).fold(0, u64::saturating_add))
}

/// Bytes used per owner, for the given users (absent: none).
pub async fn used_by_many(
    db: &impl ConnectionTrait,
    owners: &[Uuid],
) -> anyhow::Result<std::collections::HashMap<Uuid, u64>> {
    let mut used = std::collections::HashMap::new();
    if owners.is_empty() {
        return Ok(used);
    }
    let rows: Vec<(Uuid, i64)> = scopes::Entity::find()
        .select_only()
        .column(scopes::Column::OwnerId)
        .column(scopes::Column::StoredBytes)
        .filter(scopes::Column::OwnerId.is_in(owners.iter().copied()))
        .into_tuple()
        .all(db)
        .await
        .context("reading storage usage")?;
    for (owner, value) in rows {
        let total: &mut u64 = used.entry(owner).or_default();
        *total = total.saturating_add(bytes(value));
    }
    Ok(used)
}

/// `quota_exceeded` if `extra` more bytes would take the owner of `scope` over `quota`
/// (an early estimate; [`insert_charged`] enforces exactly).
pub async fn ensure_room(
    db: &impl ConnectionTrait,
    scope: &scopes::Model,
    quota: Option<u64>,
    extra: u64,
) -> Result<(), ApiError> {
    let Some(quota) = quota else {
        return Ok(());
    };
    if used_by(db, scope.owner_id).await?.saturating_add(extra) > quota {
        return Err(ApiError::QuotaExceeded);
    }
    Ok(())
}

/// Serializes the charges of one owner (see the module docs): first statement of the
/// transaction.
async fn lock_owner(db: &impl ConnectionTrait, owner: Uuid) -> anyhow::Result<()> {
    user_entity::Entity::update_many()
        .col_expr(
            user_entity::Column::QuotaBytes,
            Expr::col(user_entity::Column::QuotaBytes),
        )
        .filter(user_entity::Column::Id.eq(owner))
        .exec(db)
        .await
        .context("locking the owner's usage")?;
    Ok(())
}

/// Outcome of [`insert_charged`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charged {
    /// The row was inserted and charged.
    Inserted,
    /// The row existed: nothing charged.
    Existed,
    /// The verified object is gone (adoption only): no row inserted.
    Gone,
}

/// Inserts the row of a stored, verified object of `scope` (unless it exists) and
/// charges its stored size to the scope, in one transaction that first takes the
/// scope lock and checks the object is in place (`sync::objects`: the garbage
/// collector can't delete it meanwhile). With a `quota` (the owner's effective one)
/// the charge is refused with `quota_exceeded` if the owner's total would go over it.
/// `not_found` if the scope was purged meanwhile.
pub async fn insert_charged(
    state: &AppState,
    scope: &scopes::Model,
    object: NewObject<'_>,
    stored: Stored<'_>,
    quota: Option<u64>,
) -> Result<Charged, ApiError> {
    let (kind, hash) = match object {
        NewObject::Chunk { hash, .. } => (Kind::Chunk, hash.to_string()),
        NewObject::Manifest { hash } => (Kind::Manifest, hash.to_string()),
    };
    let size = to_i64(stored.size())?;
    let now = crate::time::now();
    let tx = state.db.begin().await.context("starting a transaction")?;
    if !objects::lock_scope(&tx, scope.id).await? {
        tx.rollback().await.context("rolling back")?;
        return Err(ApiError::NotFound);
    }
    if objects::row_exists(&tx, scope.id, kind, &hash).await? {
        tx.commit().await.context("committing")?;
        return Ok(Charged::Existed);
    }
    if !objects::ensure_present(&state.blobs, &kind.key(scope.id, &hash), stored).await? {
        tx.rollback().await.context("rolling back")?;
        return Ok(Charged::Gone);
    }
    if quota.is_some() {
        lock_owner(&tx, scope.owner_id).await?;
    }
    let inserted = match object {
        NewObject::Chunk { size: raw, .. } => chunks::Entity::insert(chunks::ActiveModel {
            scope_id: Set(scope.id),
            hash: Set(hash.clone()),
            size: Set(i32::try_from(raw).context("chunk size out of range")?),
            stored_size: Set(size),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([chunks::Column::ScopeId, chunks::Column::Hash])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await
        .context("inserting a chunk row")?,
        NewObject::Manifest { .. } => manifests::Entity::insert(manifests::ActiveModel {
            scope_id: Set(scope.id),
            hash: Set(hash.clone()),
            stored_size: Set(size),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([manifests::Column::ScopeId, manifests::Column::Hash])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await
        .context("inserting a manifest row")?,
    } == 1;
    if !inserted {
        tx.commit().await.context("committing")?;
        return Ok(Charged::Existed);
    }
    scopes::Entity::update_many()
        .col_expr(
            scopes::Column::StoredBytes,
            Expr::col(scopes::Column::StoredBytes).add(size),
        )
        .filter(scopes::Column::Id.eq(scope.id))
        .exec(&tx)
        .await
        .context("charging the scope")?;
    if let Some(quota) = quota
        && used_by(&tx, scope.owner_id).await? > quota
    {
        tx.rollback().await.context("rolling back")?;
        return Err(ApiError::QuotaExceeded);
    }
    tx.commit().await.context("committing a charged row")?;
    Ok(Charged::Inserted)
}

/// The scope's stored bytes recomputed from its `chunks` and `manifests` rows: what
/// `scopes.stored_bytes` must equal.
pub async fn recompute(db: &impl ConnectionTrait, scope: Uuid) -> anyhow::Result<u64> {
    let chunk_sizes: Vec<i64> = chunks::Entity::find()
        .select_only()
        .column(chunks::Column::StoredSize)
        .filter(chunks::Column::ScopeId.eq(scope))
        .into_tuple()
        .all(db)
        .await
        .context("reading chunk sizes")?;
    let manifest_sizes: Vec<i64> = manifests::Entity::find()
        .select_only()
        .column(manifests::Column::StoredSize)
        .filter(manifests::Column::ScopeId.eq(scope))
        .into_tuple()
        .all(db)
        .await
        .context("reading manifest sizes")?;
    Ok(chunk_sizes
        .into_iter()
        .chain(manifest_sizes)
        .map(bytes)
        .fold(0, u64::saturating_add))
}
