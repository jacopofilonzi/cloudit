//! The `chunks` table and chunk objects: lookups, verification with `cloudit-core`,
//! storing verified uploads (single or packed: the same path), adoption of objects
//! uploaded through presigned URLs.

use std::collections::HashMap;

use anyhow::Context as _;
use bytes::Bytes;
use cloudit_core::chunk::codec::decode_chunk;
use cloudit_core::error::ChunkError;
use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::MAX_STORED_CHUNK_BYTES;
use sea_orm::{
    ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _, TransactionTrait as _,
};
use uuid::Uuid;

use super::keys;
use super::objects::{self, Kind, Stored};
use super::usage::{self, Charged, NewObject};
use crate::db::entities::{chunks, scopes};
use crate::errors::ApiError;
use crate::hooks::{self, Point};
use crate::state::AppState;

/// Values per `IN (…)` list: well below SQLite's and PostgreSQL's bind limits.
pub const IN_BATCH: usize = 500;

/// The uncompressed sizes of those of `hashes` the scope has rows for.
pub async fn sizes(
    db: &impl ConnectionTrait,
    scope: Uuid,
    hashes: &[ChunkHash],
) -> anyhow::Result<HashMap<ChunkHash, u32>> {
    let mut found = HashMap::with_capacity(hashes.len());
    for batch in hashes.chunks(IN_BATCH) {
        let rows = chunks::Entity::find()
            .filter(chunks::Column::ScopeId.eq(scope))
            .filter(chunks::Column::Hash.is_in(batch.iter().map(ToString::to_string)))
            .all(db)
            .await
            .context("looking up chunks")?;
        for row in rows {
            let hash: ChunkHash = row
                .hash
                .parse()
                .with_context(|| format!("stored chunk hash {:?}", row.hash))?;
            found.insert(hash, u32::try_from(row.size).unwrap_or(0));
        }
    }
    Ok(found)
}

/// The stored sizes of those of `hashes` the scope has rows for.
pub async fn stored_sizes(
    db: &impl ConnectionTrait,
    scope: Uuid,
    hashes: &[ChunkHash],
) -> anyhow::Result<HashMap<ChunkHash, u64>> {
    let mut found = HashMap::with_capacity(hashes.len());
    for batch in hashes.chunks(IN_BATCH) {
        let rows = chunks::Entity::find()
            .filter(chunks::Column::ScopeId.eq(scope))
            .filter(chunks::Column::Hash.is_in(batch.iter().map(ToString::to_string)))
            .all(db)
            .await
            .context("looking up chunks")?;
        for row in rows {
            let hash: ChunkHash = row
                .hash
                .parse()
                .with_context(|| format!("stored chunk hash {:?}", row.hash))?;
            found.insert(hash, u64::try_from(row.stored_size).unwrap_or(u64::MAX));
        }
    }
    Ok(found)
}

/// Whether the scope has a row for `hash`.
pub async fn exists(
    db: &impl ConnectionTrait,
    scope: Uuid,
    hash: &ChunkHash,
) -> anyhow::Result<bool> {
    Ok(chunks::Entity::find_by_id((scope, hash.to_string()))
        .one(db)
        .await
        .context("looking up a chunk")?
        .is_some())
}

/// Decodes a stored chunk on the blocking pool and verifies its hash; the
/// uncompressed size on success.
pub async fn decode(stored: Bytes, hash: ChunkHash) -> anyhow::Result<Result<u32, ChunkError>> {
    tokio::task::spawn_blocking(move || {
        decode_chunk(&stored, &hash).map(|data| u32::try_from(data.len()).unwrap_or(u32::MAX))
    })
    .await
    .context("chunk decoding task")
}

/// Decodes many stored chunks in one blocking task (a pack): the uncompressed size or
/// the error of each, in order.
pub async fn decode_many(
    stored: Vec<(ChunkHash, Bytes)>,
) -> anyhow::Result<Vec<Result<u32, ChunkError>>> {
    tokio::task::spawn_blocking(move || {
        stored
            .iter()
            .map(|(hash, bytes)| {
                decode_chunk(bytes, hash).map(|data| u32::try_from(data.len()).unwrap_or(u32::MAX))
            })
            .collect()
    })
    .await
    .context("chunk decoding task")
}

/// Stores an uploaded chunk that was verified (`decode`) to be `hash` with `size`
/// uncompressed bytes: the object first, then its row, charged to the scope owner
/// (`usage::insert_charged`: scope lock, object re-check, exact quota). Idempotent.
/// The one write path of proxy uploads: a single upload calls it, a pack runs its two
/// halves ([`put_verified`] for every chunk, concurrently, then [`charge_verified`] for
/// each, one at a time so a pack holds at most one database writer).
pub async fn store_verified(
    state: &AppState,
    scope: &scopes::Model,
    quota: Option<u64>,
    hash: &ChunkHash,
    size: u32,
    stored: &Bytes,
) -> Result<(), ApiError> {
    put_verified(state, scope.id, hash, stored).await?;
    charge_verified(state, scope, quota, hash, size, stored).await
}

/// First half of [`store_verified`]: the object.
pub async fn put_verified(
    state: &AppState,
    scope: Uuid,
    hash: &ChunkHash,
    stored: &Bytes,
) -> Result<(), ApiError> {
    state
        .blobs
        .put(&keys::chunk(scope, hash), stored.clone())
        .await?;
    hooks::reached(Point::UploadStored, &scope.to_string()).await;
    Ok(())
}

/// Second half of [`store_verified`]: the row, charged under the scope lock (the object
/// is checked again and stored again if a collector removed it meanwhile).
pub async fn charge_verified(
    state: &AppState,
    scope: &scopes::Model,
    quota: Option<u64>,
    hash: &ChunkHash,
    size: u32,
    stored: &Bytes,
) -> Result<(), ApiError> {
    usage::insert_charged(
        state,
        scope,
        NewObject::Chunk { hash, size },
        Stored::Bytes(stored),
        quota,
    )
    .await?;
    Ok(())
}

/// Outcome of [`adopt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    /// A verified row exists now; the uncompressed size.
    Present(u32),
    /// No object.
    Missing,
    /// The object doesn't decode to the hash (deleted unless a row appeared meanwhile).
    Invalid,
}

/// Turns an object stored without a row (typically uploaded through a presigned URL)
/// into a verified, charged row: reads it (bounded), decodes it, checks the hash. An
/// invalid object is deleted, unless a row for it appeared meanwhile (a concurrent
/// verified upload of the same chunk).
pub async fn adopt(
    state: &AppState,
    scope: &scopes::Model,
    quota: Option<u64>,
    hash: &ChunkHash,
) -> Result<Adopted, ApiError> {
    let key = keys::chunk(scope.id, hash);
    let Some(meta) = state.blobs.head(&key).await? else {
        return Ok(Adopted::Missing);
    };
    let valid = if usize::try_from(meta.size).map_or(true, |n| n > MAX_STORED_CHUNK_BYTES) {
        None
    } else {
        match state.blobs.get_bytes(&key).await? {
            None => return Ok(Adopted::Missing),
            Some(stored) if stored.len() as u64 == meta.size => decode(stored, *hash).await?.ok(),
            // Replaced between HEAD and GET: missing for now, the client uploads again.
            Some(_) => return Ok(Adopted::Missing),
        }
    };
    match valid {
        Some(size) => {
            hooks::reached(Point::AdoptVerified, &scope.id.to_string()).await;
            match usage::insert_charged(
                state,
                scope,
                NewObject::Chunk { hash, size },
                Stored::Verified(&meta),
                quota,
            )
            .await?
            {
                Charged::Gone => Ok(Adopted::Missing),
                Charged::Inserted | Charged::Existed => Ok(Adopted::Present(size)),
            }
        }
        None => {
            // Deleted under the scope lock (`sync::objects`), unless a verified upload
            // of the same chunk inserted its row meanwhile: the object is theirs now.
            let tx = state.db.begin().await.context("starting a transaction")?;
            let locked = objects::lock_scope(&tx, scope.id).await?;
            if locked && objects::row_exists(&tx, scope.id, Kind::Chunk, &hash.to_string()).await? {
                tx.commit().await.context("committing")?;
                return Ok(Adopted::Present(
                    sizes(&state.db, scope.id, std::slice::from_ref(hash))
                        .await?
                        .get(hash)
                        .copied()
                        .unwrap_or(0),
                ));
            }
            state.blobs.delete(&key).await?;
            tx.commit().await.context("committing")?;
            tracing::warn!(scope_id = %scope.id, chunk = %hash, "deleted an invalid uploaded chunk");
            Ok(Adopted::Invalid)
        }
    }
}
