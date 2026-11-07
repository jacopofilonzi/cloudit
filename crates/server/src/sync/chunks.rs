//! The `chunks` table and chunk objects: lookups, verification with `cloudit-core`,
//! adoption of objects uploaded through presigned URLs.

use std::collections::HashMap;

use anyhow::Context as _;
use bytes::Bytes;
use cloudit_core::chunk::codec::decode_chunk;
use cloudit_core::error::ChunkError;
use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::MAX_STORED_CHUNK_BYTES;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use uuid::Uuid;

use super::keys;
use super::usage::{self, NewObject};
use crate::db::entities::{chunks, scopes};
use crate::errors::ApiError;
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
            Some(stored) => {
                let stored_size = stored.len() as u64;
                decode(stored, *hash)
                    .await?
                    .ok()
                    .map(|size| (size, stored_size))
            }
        }
    };
    match valid {
        Some((size, stored_size)) => {
            usage::insert_charged(
                &state.db,
                scope,
                NewObject::Chunk { hash, size },
                stored_size,
                quota,
            )
            .await?;
            Ok(Adopted::Present(size))
        }
        None => {
            if exists(&state.db, scope.id, hash).await? {
                // Verified by someone else meanwhile: the object is theirs now.
                return Ok(Adopted::Present(
                    sizes(&state.db, scope.id, std::slice::from_ref(hash))
                        .await?
                        .get(hash)
                        .copied()
                        .unwrap_or(0),
                ));
            }
            state.blobs.delete(&key).await?;
            tracing::warn!(scope_id = %scope.id, chunk = %hash, "deleted an invalid uploaded chunk");
            Ok(Adopted::Invalid)
        }
    }
}
