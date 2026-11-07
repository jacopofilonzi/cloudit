//! `/api/v1/folders/{id}/snapshots` and `/api/v1/snapshots/{id}*`: commits, conflicts
//! and their resolution, history, manifests, pin and label, restores, ZIP downloads. Session or device token; reading needs viewer, everything
//! that writes editor (`scopes::access`).

use std::collections::BTreeMap;

use anyhow::Context as _;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{EntryKind, Manifest};
use cloudit_core::protocol::{
    CommitResponse, Patch, RestoreSnapshotRequest, ScopeRole, SnapshotInfo, SnapshotKind,
    SnapshotList, UpdateSnapshotRequest,
};
use futures::{StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use uuid::Uuid;

use super::extract::{ApiJson, body_bytes, optional_json, parse_id};
use crate::auth::extractors::AnyUser;
use crate::db::entities::{folders as folder_entity, scopes, snapshots as snapshot_entity};
use crate::errors::ApiError;
use crate::scopes::access::{FolderAccess, SnapshotAccess};
use crate::state::AppState;
use crate::sync::chunks::{self, Adopted};
use crate::sync::snapshots::{self, ListFilter, NewSnapshot};
use crate::sync::{archive, manifests, usage};
use crate::time::{rfc3339, rfc3339_opt};
use crate::validate;

/// Most hashes listed in a `chunk_missing` error.
const MAX_LISTED: usize = 1000;
/// Objects verified at once during a commit (uploads through presigned URLs).
const VERIFY_CONCURRENCY: usize = 8;

pub fn snapshot_info(
    s: &snapshot_entity::Model,
    head: Option<Uuid>,
) -> anyhow::Result<SnapshotInfo> {
    Ok(SnapshotInfo {
        id: s.id.to_string(),
        folder_id: s.folder_id.to_string(),
        parent_id: s.parent_id.map(|id| id.to_string()),
        kind: s.kind(),
        manifest_hash: snapshots::manifest_hash(s)?,
        file_count: usage::bytes(s.file_count),
        logical_size: usage::bytes(s.logical_size),
        author_id: s.author_id.to_string(),
        device_id: s.device_id.map(|id| id.to_string()),
        created_at: rfc3339(s.created_at),
        pinned: s.pinned,
        label: s.label.clone(),
        restored_from: s.restored_from.map(|id| id.to_string()),
        conflict_resolved_at: rfc3339_opt(s.conflict_resolved_at),
        is_head: head == Some(s.id),
    })
}

/// The folder's head as it is now.
async fn current_head(state: &AppState, folder: Uuid) -> Result<Option<Uuid>, ApiError> {
    Ok(crate::scopes::folders::find(&state.db, folder)
        .await?
        .and_then(|f| f.head_snapshot_id))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitParams {
    /// A snapshot id of the folder, or `none`.
    parent: String,
    kind: Option<SnapshotKind>,
}

/// The distinct chunks a manifest references, with their sizes.
fn referenced(manifest: &Manifest) -> BTreeMap<ChunkHash, u32> {
    manifest
        .entries
        .iter()
        .flat_map(|e| e.chunks.iter())
        .map(|c| (c.hash, c.size))
        .collect()
}

/// Makes sure the scope has a verified row for every referenced chunk: rows first,
/// then objects without a row (presigned uploads) are verified and adopted.
async fn ensure_chunks(
    state: &AppState,
    scope: &scopes::Model,
    quota: Option<u64>,
    wanted: &BTreeMap<ChunkHash, u32>,
) -> Result<(), ApiError> {
    let hashes: Vec<ChunkHash> = wanted.keys().copied().collect();
    let mut present = chunks::sizes(&state.db, scope.id, &hashes).await?;
    let absent: Vec<ChunkHash> = hashes
        .into_iter()
        .filter(|h| !present.contains_key(h))
        .collect();
    let outcomes: Vec<(ChunkHash, Adopted)> = futures::stream::iter(absent)
        .map(|hash| async move {
            chunks::adopt(state, scope, quota, &hash)
                .await
                .map(|outcome| (hash, outcome))
        })
        .buffer_unordered(VERIFY_CONCURRENCY)
        .try_collect()
        .await?;
    let mut missing = Vec::new();
    let mut invalid = Vec::new();
    for (hash, outcome) in outcomes {
        match outcome {
            Adopted::Present(size) => {
                present.insert(hash, size);
            }
            Adopted::Missing => missing.push(hash),
            Adopted::Invalid => invalid.push(hash),
        }
    }
    if !invalid.is_empty() {
        invalid.sort_unstable();
        return Err(ApiError::ChunkInvalid(invalid));
    }
    if !missing.is_empty() {
        missing.sort_unstable();
        missing.truncate(MAX_LISTED);
        return Err(ApiError::ChunkMissing(missing));
    }
    if wanted
        .iter()
        .any(|(hash, size)| present.get(hash) != Some(size))
    {
        return Err(ApiError::ManifestInvalid("manifest_chunk_size_wrong"));
    }
    Ok(())
}

/// `POST /folders/{id}/snapshots?parent=<snapshot id|none>&kind=normal|conflict`
/// (editor), body = the stored manifest bytes (zstd canonical JSON, decompressed at
/// most 64 MiB). See STRUCTURE.md "Sync protocol" for the full flow.
pub async fn commit(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    params: Result<Query<CommitParams>, QueryRejection>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<CommitResponse>), ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    let Query(params) = params.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let kind = params.kind.unwrap_or(SnapshotKind::Normal);
    if kind == SnapshotKind::Restore {
        return Err(ApiError::validation("kind", "normal or conflict"));
    }
    let folder: &folder_entity::Model = &access.folder;
    let scope = &access.scope.scope;
    let parent = match params.parent.as_str() {
        "none" => None,
        raw => {
            let found = match raw.parse::<Uuid>() {
                Ok(id) => snapshots::find(&state.db, id).await?,
                Err(_) => None,
            };
            match found {
                Some(s) if s.folder_id == folder.id => Some(s.id),
                _ => {
                    return Err(ApiError::validation(
                        "parent",
                        "not a snapshot of this folder",
                    ));
                }
            }
        }
    };
    let stored = body_bytes(body)?;
    let (manifest, hash) = manifests::decode(stored.clone())
        .await?
        .map_err(|err| ApiError::ManifestInvalid(err.code()))?;

    if kind == SnapshotKind::Normal
        && let Some(parent) = parent
        && folder.head_snapshot_id == Some(parent)
        && let Some(head) = snapshots::find(&state.db, parent).await?
        && snapshots::manifest_hash(&head)? == hash
    {
        return Ok((
            StatusCode::OK,
            Json(CommitResponse {
                unchanged: true,
                snapshot: snapshot_info(&head, Some(head.id))?,
            }),
        ));
    }

    let quota = usage::owner_quota(&state.db, scope).await?;
    let wanted = referenced(&manifest);
    ensure_chunks(&state, scope, quota, &wanted).await?;
    manifests::store(&state, scope, quota, &hash, stored).await?;

    let new = NewSnapshot {
        folder: folder.id,
        scope: scope.id,
        parent,
        kind,
        manifest_hash: hash,
        file_count: manifest
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .count() as u64,
        logical_size: manifest.logical_size(),
        author: me.user.id,
        device: me.device_id,
        restored_from: None,
    };
    let hashes: Vec<ChunkHash> = wanted.keys().copied().collect();
    let snapshot = match snapshots::commit(&state.db, &new, &hashes).await {
        Ok(snapshot) => snapshot,
        Err(ApiError::Internal(err)) => {
            // A chunk row may have been garbage-collected meanwhile (its foreign key
            // failed): report it like any missing chunk.
            let present = chunks::sizes(&state.db, scope.id, &hashes).await?;
            let mut gone: Vec<ChunkHash> = hashes
                .into_iter()
                .filter(|h| !present.contains_key(h))
                .collect();
            if gone.is_empty() {
                return Err(ApiError::Internal(err));
            }
            gone.truncate(MAX_LISTED);
            return Err(ApiError::ChunkMissing(gone));
        }
        Err(other) => return Err(other),
    };
    tracing::info!(
        user_id = %me.user.id,
        folder_id = %folder.id,
        snapshot_id = %snapshot.id,
        kind = kind.as_str(),
        "snapshot committed"
    );
    let head = (kind == SnapshotKind::Normal).then_some(snapshot.id);
    Ok((
        StatusCode::CREATED,
        Json(CommitResponse {
            unchanged: false,
            snapshot: snapshot_info(&snapshot, head)?,
        }),
    ))
}

/// `GET /snapshots/{id}` (viewer).
pub async fn get(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<SnapshotInfo>, ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    Ok(Json(snapshot_info(
        &access.snapshot,
        access.folder.folder.head_snapshot_id,
    )?))
}

/// `POST /snapshots/{id}/resolve` (editor, conflicts only): marks the conflict resolved
/// and keeps the server version ("keep server"). Idempotent.
pub async fn resolve(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<SnapshotInfo>, ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    if !snapshots::resolve(&state.db, access.snapshot.id).await? {
        return Err(ApiError::validation("kind", "not a conflict snapshot"));
    }
    let snapshot = snapshots::find(&state.db, access.snapshot.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let head = current_head(&state, snapshot.folder_id).await?;
    Ok(Json(snapshot_info(&snapshot, head)?))
}

/// Default and maximum page size of the history.
const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;
/// Longest label, in characters.
const MAX_LABEL_CHARS: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// The last id of the previous page.
    cursor: Option<String>,
    limit: Option<u64>,
    kind: Option<SnapshotKind>,
    /// `true`: only conflicts not resolved yet.
    unresolved: Option<bool>,
}

/// `GET /folders/{id}/snapshots?cursor=&limit=&kind=&unresolved=` (viewer): newest
/// first; `next_cursor` continues with older ones.
pub async fn list(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Json<SnapshotList>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let Query(params) = params.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::validation("limit", format!("1 to {MAX_LIMIT}")));
    }
    let before = params
        .cursor
        .as_deref()
        .map(str::parse::<Uuid>)
        .transpose()
        .map_err(|_| ApiError::validation("cursor", "not a snapshot id"))?;
    let filter = ListFilter {
        before,
        kind: params.kind,
        unresolved: params.unresolved.unwrap_or(false),
    };
    let mut page = snapshots::list(&state.db, access.folder.id, filter, limit + 1).await?;
    let more = page.len() as u64 > limit;
    page.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let head = access.folder.head_snapshot_id;
    Ok(Json(SnapshotList {
        next_cursor: if more {
            page.last().map(|s| s.id.to_string())
        } else {
            None
        },
        snapshots: page
            .iter()
            .map(|s| snapshot_info(s, head))
            .collect::<anyhow::Result<_>>()?,
    }))
}

/// Answer format of the manifest download.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestFormat {
    /// The stored bytes (zstd canonical JSON), for clients.
    #[default]
    Stored,
    /// The canonical JSON, for the web UI.
    Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestParams {
    format: Option<ManifestFormat>,
}

/// `GET /snapshots/{id}/manifest[?format=json]` (viewer): the stored manifest bytes
/// (`application/octet-stream`), or the canonical JSON. `ETag` = the quoted manifest
/// hash; `If-None-Match` with it → 304.
pub async fn manifest(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    params: Result<Query<ManifestParams>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let Query(params) = params.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let hash = snapshots::manifest_hash(&access.snapshot)?;
    let etag = format!("\"{hash}\"");
    let cache = [
        (header::ETAG, etag.clone()),
        (header::CACHE_CONTROL, "private, no-cache".to_owned()),
    ];
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    let scope = access.snapshot.scope_id;
    match params.format.unwrap_or_default() {
        ManifestFormat::Stored => {
            let stored = manifests::stored_bytes(&state, scope, &hash).await?;
            Ok((
                cache,
                [(header::CONTENT_TYPE, "application/octet-stream")],
                stored,
            )
                .into_response())
        }
        ManifestFormat::Json => {
            let manifest = manifests::load(&state, scope, &hash).await?;
            let json = manifest.canonical_json().context("encoding a manifest")?;
            Ok((cache, [(header::CONTENT_TYPE, "application/json")], json).into_response())
        }
    }
}

/// `PATCH /snapshots/{id} {pinned?, label?}` (editor).
pub async fn update(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateSnapshotRequest>,
) -> Result<Json<SnapshotInfo>, ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    let label = match body.label {
        Patch::Missing => None,
        Patch::Null => Some(None),
        Patch::Value(raw) if raw.trim().is_empty() => Some(None),
        Patch::Value(raw) => Some(Some(validate::text(&raw, "label", MAX_LABEL_CHARS)?)),
    };
    snapshots::update(&state.db, access.snapshot.id, body.pinned, label).await?;
    let snapshot = snapshots::find(&state.db, access.snapshot.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let head = current_head(&state, snapshot.folder_id).await?;
    Ok(Json(snapshot_info(&snapshot, head)?))
}

/// `POST /snapshots/{id}/restore {expected_head?}` (editor) → 201: a `restore`
/// snapshot with the same content becomes the head. With `expected_head`, `conflict`
/// if the head is another one. Restoring a conflict snapshot resolves it ("keep
/// local").
pub async fn restore(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<SnapshotInfo>), ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    let body: RestoreSnapshotRequest = optional_json(&body)?;
    let head = access.folder.folder.head_snapshot_id;
    if let Some(expected) = body.expected_head {
        let expected = expected
            .parse::<Uuid>()
            .map_err(|_| ApiError::validation("expected_head", "not a snapshot id"))?;
        if head != Some(expected) {
            return Err(ApiError::Conflict { head });
        }
    }
    let snapshot =
        snapshots::restore(&state.db, &access.snapshot, head, me.user.id, me.device_id).await?;
    tracing::info!(
        user_id = %me.user.id,
        folder_id = %snapshot.folder_id,
        snapshot_id = %snapshot.id,
        restored_from = %access.snapshot.id,
        "snapshot restored"
    );
    Ok((
        StatusCode::CREATED,
        Json(snapshot_info(&snapshot, Some(snapshot.id))?),
    ))
}

/// `GET /snapshots/{id}/archive` (viewer, session or device token): the snapshot's
/// files as a ZIP, streamed (`sync::archive`), named `<folder>-<time>.zip`.
pub async fn archive(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let access = SnapshotAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let snapshot = &access.snapshot;
    let hash = snapshots::manifest_hash(snapshot)?;
    let manifest = manifests::load(&state, snapshot.scope_id, &hash).await?;
    let name = archive::filename(&access.folder.folder.name, snapshot.created_at);
    tracing::info!(user_id = %me.user.id, snapshot_id = %snapshot.id, "zip download");
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                archive::content_disposition(&name),
            ),
        ],
        archive::body(state.clone(), snapshot.scope_id, manifest),
    )
        .into_response())
}
