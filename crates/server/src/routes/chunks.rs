//! `/api/v1/folders/{id}/chunks*`: which chunks the folder's scope lacks, uploading
//! them through the server (proxy) and downloading them. Session or device token;
//! check and upload need editor, downloads viewer. Presigned URLs (when `S3_PRESIGN`
//! is on) bypass the proxy routes: objects uploaded that way get their row when a
//! commit verifies them (`sync::chunks::adopt`). Chunk packs (`cloudit_core::pack`,
//! API level 2) move many small chunks in one request; they are unpacked here and
//! never stored as such.

use std::collections::BTreeMap;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use bytes::BytesMut;
use cloudit_core::chunk::MAX_CHUNK_SIZE;
use cloudit_core::hash::ChunkHash;
use cloudit_core::pack::{
    MAX_PACK_BYTES, MAX_PACK_RECORDS, PACK_CONTENT_TYPE, PACK_VERSION, RECORD_HEADER_BYTES,
    decode_pack, record_header,
};
use cloudit_core::protocol::{
    ChunkCheckRequest, ChunkCheckResponse, ChunkDownload, ChunkDownloadRequest,
    ChunkDownloadResponse, ChunkPackRequest, ChunkPackUploadResponse, MAX_CHUNKS_PER_REQUEST,
    MAX_STORED_CHUNK_BYTES, MissingChunk, ScopeRole,
};
use futures::StreamExt as _;

use super::extract::{ApiJson, body_bytes, parse_id};
use crate::auth::extractors::AnyUser;
use crate::errors::ApiError;
use crate::scopes::access::FolderAccess;
use crate::state::AppState;
use crate::sync::transfer::Transfers;
use crate::sync::usage;
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
    chunks::store_verified(&state, scope, quota, &hash, size, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /folders/{id}/chunks/pack` (editor; API level 2), body = a chunk pack
/// (`cloudit_core::pack`, at most `MAX_PACK_BYTES` and `MAX_PACK_RECORDS`): every
/// record is decoded and verified first, so an invalid one (`chunk_invalid` with every
/// bad hash) stores nothing; then the chunks the scope lacks are stored exactly like
/// [`upload`] (objects [`PACK_STORE_CONCURRENCY`] at a time, rows one by one), after one quota
/// estimate for all of them. Idempotent. A charge refused midway (`quota_exceeded`)
/// leaves the chunks stored before it: each is a complete, charged object.
pub async fn upload_pack(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<ChunkPackUploadResponse>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    let body = body_bytes(body)?;
    let entries =
        decode_pack(&body).map_err(|e| ApiError::BadRequest(format!("{}: {e}", e.code())))?;
    let records: Vec<(ChunkHash, Bytes)> = entries
        .into_iter()
        .map(|entry| (entry.hash, body.slice(entry.range)))
        .collect();
    let decoded = chunks::decode_many(records.clone()).await?;
    let invalid: Vec<ChunkHash> = records
        .iter()
        .zip(&decoded)
        .filter(|(_, result)| result.is_err())
        .map(|((hash, _), _)| *hash)
        .collect();
    if !invalid.is_empty() {
        return Err(ApiError::ChunkInvalid(invalid));
    }
    let scope = &access.scope.scope;
    let hashes: Vec<ChunkHash> = records.iter().map(|(hash, _)| *hash).collect();
    let present = chunks::sizes(&state.db, scope.id, &hashes).await?;
    let new: Vec<(ChunkHash, u32, Bytes)> = records
        .into_iter()
        .zip(decoded)
        .filter(|((hash, _), _)| !present.contains_key(hash))
        .filter_map(|((hash, bytes), size)| size.ok().map(|size| (hash, size, bytes)))
        .collect();
    let answer = ChunkPackUploadResponse {
        chunks: u32::try_from(hashes.len()).unwrap_or(u32::MAX),
        stored: u32::try_from(new.len()).unwrap_or(u32::MAX),
    };
    if new.is_empty() {
        return Ok(Json(answer));
    }
    let quota = usage::owner_quota(&state.db, scope).await?;
    let estimate = new.iter().map(|(_, _, bytes)| bytes.len() as u64).sum();
    usage::ensure_room(&state.db, scope, quota, estimate).await?;
    let state = &state;
    // Objects in parallel; rows one at a time as their objects land, so a pack is one
    // database writer (SQLite has one writer: 8 per pack starved other requests past
    // the busy timeout). Owned values moved into each future (a closure over borrowed
    // items isn't `Send` for every lifetime).
    let mut puts = futures::stream::iter(new.into_iter().map(|(hash, size, bytes)| async move {
        chunks::put_verified(state, scope.id, &hash, &bytes).await?;
        Ok::<_, ApiError>((hash, size, bytes))
    }))
    .buffer_unordered(PACK_STORE_CONCURRENCY);
    while let Some(put) = puts.next().await {
        let (hash, size, bytes) = put?;
        chunks::charge_verified(state, scope, quota, &hash, size, &bytes).await?;
    }
    Ok(Json(answer))
}

/// Objects of one uploaded pack stored at the same time.
const PACK_STORE_CONCURRENCY: usize = 8;
/// Objects of one downloaded pack read ahead.
const PACK_PREFETCH: usize = 8;

/// `POST /folders/{id}/chunks/pack/download {hashes}` (viewer; API level 2; at most
/// `MAX_PACK_RECORDS` hashes): a chunk pack of the stored chunk bytes of the requested
/// chunks the scope has, sorted by hash, streamed as the objects are read. Records stop
/// before the body would pass `MAX_PACK_BYTES`; the requested hashes without a record
/// (not in the scope, or past the limit) are for `chunks/download`. A failure while
/// streaming aborts the body (never a complete-looking pack with a bad record).
pub async fn download_pack(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ChunkPackRequest>,
) -> Result<Response, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    if body.hashes.len() > MAX_PACK_RECORDS {
        return Err(ApiError::validation(
            "hashes",
            format!("at most {MAX_PACK_RECORDS} entries"),
        ));
    }
    let mut hashes = body.hashes;
    hashes.sort_unstable();
    hashes.dedup();
    let scope = access.scope.scope.id;
    let present = chunks::stored_sizes(&state.db, scope, &hashes).await?;
    // The rows' stored sizes plan the records; the bytes actually read are checked
    // again while streaming (an object can be replaced through a presigned URL).
    let mut planned = Vec::new();
    let mut total = 1usize;
    for hash in hashes {
        let Some(&stored) = present.get(&hash) else {
            continue;
        };
        let next = usize::try_from(stored)
            .ok()
            .and_then(|s| total.checked_add(RECORD_HEADER_BYTES + s))
            .filter(|n| *n <= MAX_PACK_BYTES);
        let Some(next) = next else { break };
        total = next;
        planned.push(hash);
    }
    let objects = futures::stream::iter(planned)
        .map(move |hash| {
            let state = state.clone();
            async move {
                let bytes = state
                    .blobs
                    .get_bytes(&keys::chunk(scope, &hash))
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!("chunk {hash} of scope {scope} has a row but no object")
                    })?;
                Ok::<_, anyhow::Error>((hash, bytes))
            }
        })
        .buffered(PACK_PREFETCH)
        .boxed();
    let records = futures::stream::unfold(Some((objects, 1usize)), move |stage| async move {
        let (mut objects, total) = stage?;
        let (hash, bytes) = match objects.next().await? {
            Ok(object) => object,
            Err(error) => {
                tracing::error!(?error, "chunk pack download failed");
                return Some((Err(std::io::Error::other("chunk pack failed")), None));
            }
        };
        let Some(len) = u32::try_from(bytes.len())
            .ok()
            .filter(|_| !bytes.is_empty() && bytes.len() <= MAX_STORED_CHUNK_BYTES)
        else {
            tracing::error!(chunk = %hash, %scope, len = bytes.len(), "stored chunk has an invalid size");
            return Some((Err(std::io::Error::other("chunk pack failed")), None));
        };
        if total + RECORD_HEADER_BYTES + bytes.len() > MAX_PACK_BYTES {
            // Larger than its row said: it and the rest are left to `chunks/download`.
            return None;
        }
        let mut record = BytesMut::with_capacity(RECORD_HEADER_BYTES + bytes.len());
        record.extend_from_slice(&record_header(&hash, len));
        record.extend_from_slice(&bytes);
        let total = total + record.len();
        Some((Ok(record.freeze()), Some((objects, total))))
    });
    let body =
        futures::stream::once(async { Ok(Bytes::from_static(&[PACK_VERSION])) }).chain(records);
    Ok((
        [(header::CONTENT_TYPE, PACK_CONTENT_TYPE)],
        Body::from_stream(body),
    )
        .into_response())
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
