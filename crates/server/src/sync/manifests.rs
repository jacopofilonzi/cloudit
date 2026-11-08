//! The `manifests` table and manifest objects: decoding with `cloudit-core` (bounded,
//! on the blocking pool), storing a new one, loading one back.

use anyhow::Context as _;
use bytes::Bytes;
use cloudit_core::error::ManifestError;
use cloudit_core::hash::ManifestHash;
use cloudit_core::manifest::{DEFAULT_MAX_MANIFEST_BYTES, Manifest, decode_manifest};
use sea_orm::{ConnectionTrait, EntityTrait as _};
use uuid::Uuid;

use super::keys;
use super::objects::Stored;
use super::usage::{self, NewObject};
use crate::db::entities::{manifests, scopes};
use crate::errors::ApiError;
use crate::state::AppState;

/// Decompresses, parses and validates a stored manifest (decompressed size bounded by
/// `DEFAULT_MAX_MANIFEST_BYTES`).
pub async fn decode(
    stored: Bytes,
) -> anyhow::Result<Result<(Manifest, ManifestHash), ManifestError>> {
    tokio::task::spawn_blocking(move || decode_manifest(&stored, DEFAULT_MAX_MANIFEST_BYTES))
        .await
        .context("manifest decoding task")
}

pub async fn exists(
    db: &impl ConnectionTrait,
    scope: Uuid,
    hash: &ManifestHash,
) -> anyhow::Result<bool> {
    Ok(manifests::Entity::find_by_id((scope, hash.to_string()))
        .one(db)
        .await
        .context("looking up a manifest")?
        .is_some())
}

/// Stores a decoded manifest's bytes and its charged row, unless the scope has it.
pub async fn store(
    state: &AppState,
    scope: &scopes::Model,
    quota: Option<u64>,
    hash: &ManifestHash,
    stored: Bytes,
) -> Result<(), ApiError> {
    if exists(&state.db, scope.id, hash).await? {
        return Ok(());
    }
    let size = stored.len() as u64;
    usage::ensure_room(&state.db, scope, quota, size).await?;
    state
        .blobs
        .put(&keys::manifest(scope.id, hash), stored.clone())
        .await?;
    usage::insert_charged(
        state,
        scope,
        NewObject::Manifest { hash },
        Stored::Bytes(&stored),
        quota,
    )
    .await?;
    Ok(())
}

/// The stored bytes of a manifest the scope has a row for.
pub async fn stored_bytes(
    state: &AppState,
    scope: Uuid,
    hash: &ManifestHash,
) -> anyhow::Result<Bytes> {
    state
        .blobs
        .get_bytes(&keys::manifest(scope, hash))
        .await?
        .with_context(|| format!("manifest {hash} of scope {scope} has a row but no object"))
}

/// Loads and decodes a stored manifest (it was validated when stored; decoding again
/// guards against a damaged object).
pub async fn load(state: &AppState, scope: Uuid, hash: &ManifestHash) -> anyhow::Result<Manifest> {
    let stored = stored_bytes(state, scope, hash).await?;
    let (manifest, actual) = decode(stored)
        .await?
        .with_context(|| format!("stored manifest {hash} of scope {scope}"))?;
    anyhow::ensure!(actual == *hash, "stored manifest {hash} hashes to {actual}");
    Ok(manifest)
}
