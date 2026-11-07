//! `/api/v1/folders/{id}/chunks*`: which chunks the folder's scope lacks, uploading
//! them through the server (proxy) and downloading them. Session or device token;
//! check and upload need editor, downloads viewer. Presigned URLs (when `S3_PRESIGN`
//! is on) bypass the proxy routes: objects uploaded that way get their row when a
//! commit verifies them (`sync::chunks::adopt`).

use std::collections::BTreeMap;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::chunk::MAX_CHUNK_SIZE;
use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::{
    ChunkCheckRequest, ChunkCheckResponse, ChunkDownload, ChunkDownloadRequest,
    ChunkDownloadResponse, MAX_CHUNKS_PER_REQUEST, MissingChunk, ScopeRole,
};
use futures::StreamExt as _;

use super::extract::{ApiJson, body_bytes, parse_id};
use crate::auth::extractors::AnyUser;
use crate::errors::ApiError;
use crate::scopes::access::FolderAccess;
use crate::state::AppState;
use crate::sync::transfer::Transfers;
use crate::sync::usage::{self, NewObject};
use crate::sync::{chunks, keys};

/// A chunk hash from the path; anything else is `not_found`.
fn parse_hash(raw: &str) -> Result<ChunkHash, ApiError> {
    raw.parse().map_err(|_| ApiError::NotFound)
}

fn ensure_count(n: usize, field: &'static str) -> Result<(), ApiError> {
    if n > MAX_CHUNKS_PER_REQUEST {
        return Err(ApiError::validation(
            field,
            format!("at most {MAX_CHUNKS_PER_REQUEST} entries"),
        ));
    }
    Ok(())
}

/// `POST /folders/{id}/chunks/check {chunks: [{hash, size}]}` (editor): the chunks
/// the scope lacks, each with where to upload it. `quota_exceeded` if the owner's
/// usage plus the uncompressed sizes of the missing chunks would exceed the quota (an
/// estimate: the upload itself charges the exact stored size).
pub async fn check(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ChunkCheckRequest>,
) -> Result<Json<ChunkCheckResponse>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    ensure_count(body.chunks.len(), "chunks")?;
    let mut wanted: BTreeMap<ChunkHash, u32> = BTreeMap::new();
    for chunk in &body.chunks {
        if chunk.size == 0 || chunk.size > MAX_CHUNK_SIZE {
            return Err(ApiError::validation("chunks", "size must be 1 to 1 MiB"));
        }
        if wanted
            .insert(chunk.hash, chunk.size)
            .is_some_and(|s| s != chunk.size)
        {
            return Err(ApiError::validation(
                "chunks",
                "one hash listed with different sizes",
            ));
        }
    }
    let scope = &access.scope.scope;
    let hashes: Vec<ChunkHash> = wanted.keys().copied().collect();
    let present = chunks::sizes(&state.db, scope.id, &hashes).await?;
    let missing: Vec<(ChunkHash, u32)> = wanted
        .into_iter()
        .filter(|(hash, _)| !present.contains_key(hash))
        .collect();
    let estimate = missing.iter().map(|(_, size)| u64::from(*size)).sum();
    let quota = usage::owner_quota(&state.db, scope).await?;
    usage::ensure_room(&state.db, scope, quota, estimate).await?;

    let transfers = Transfers::new(&state, scope.id, access.folder.id).await?;
    let mut out = Vec::with_capacity(missing.len());
    for (hash, _) in missing {
        out.push(MissingChunk {
            hash,
            upload: transfers.upload(&hash).await?,
        });
    }
    Ok(Json(ChunkCheckResponse { missing: out }))
}

/// `PUT /folders/{id}/chunks/{hash}` (editor), body = the stored chunk bytes (at most
/// `MAX_STORED_CHUNK_BYTES`): decoded and verified (`chunk_invalid` if it doesn't
/// decode to `hash`), stored, then its row inserted and charged to the scope owner
/// (`quota_exceeded`). Idempotent: 204 whether it was new or not.
pub async fn upload(
    State(state): State<AppState>,
    me: AnyUser,
    Path((id, hash)): Path<(String, String)>,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    let hash = parse_hash(&hash)?;
    let body = body_bytes(body)?;
    let scope = &access.scope.scope;
    let stored_size = body.len() as u64;
    let Ok(size) = chunks::decode(body.clone(), hash).await? else {
        return Err(ApiError::ChunkInvalid(vec![hash]));
    };
    if chunks::exists(&state.db, scope.id, &hash).await? {
        return Ok(StatusCode::NO_CONTENT);
    }
    let quota = usage::owner_quota(&state.db, scope).await?;
    usage::ensure_room(&state.db, scope, quota, stored_size).await?;
    state.blobs.put(&keys::chunk(scope.id, &hash), body).await?;
    usage::insert_charged(
        &state.db,
        scope,
        NewObject::Chunk { hash: &hash, size },
        stored_size,
        quota,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /folders/{id}/chunks/download {hashes}` (viewer): where to download each
/// chunk the scope has; the others are listed as `missing`.
pub async fn download_urls(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ChunkDownloadRequest>,
) -> Result<Json<ChunkDownloadResponse>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    ensure_count(body.hashes.len(), "hashes")?;
    let mut hashes = body.hashes;
    hashes.sort_unstable();
    hashes.dedup();
    let scope = access.scope.scope.id;
    let present = chunks::sizes(&state.db, scope, &hashes).await?;
    let transfers = Transfers::new(&state, scope, access.folder.id).await?;
    let mut out = ChunkDownloadResponse {
        chunks: Vec::with_capacity(present.len()),
        missing: Vec::new(),
    };
    for hash in hashes {
        if present.contains_key(&hash) {
            out.chunks.push(ChunkDownload {
                hash,
                download: transfers.download(&hash).await?,
            });
        } else {
            out.missing.push(hash);
        }
    }
    Ok(Json(out))
}

/// `GET /folders/{id}/chunks/{hash}` (viewer): streams the stored chunk bytes (the
/// client decodes and verifies them). 404 unless the scope has the chunk.
pub async fn download(
    State(state): State<AppState>,
    me: AnyUser,
    Path((id, hash)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let hash = parse_hash(&hash)?;
    let scope = access.scope.scope.id;
    if !chunks::exists(&state.db, scope, &hash).await? {
        return Err(ApiError::NotFound);
    }
    let stream = state
        .blobs
        .get(&keys::chunk(scope, &hash))
        .await?
        .ok_or_else(|| anyhow::anyhow!("chunk {hash} of scope {scope} has a row but no object"))?;
    let body = Body::from_stream(stream.map(|part| part.map_err(std::io::Error::other)));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        body,
    )
        .into_response())
}
