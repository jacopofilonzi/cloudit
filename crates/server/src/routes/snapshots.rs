//! `/api/v1/folders/{id}/snapshots` and `/api/v1/snapshots/{id}*`: commits, conflicts
//! and their resolution. Session or device token; reading needs viewer, everything
//! that writes editor (`scopes::access`).

use std::collections::BTreeMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{EntryKind, Manifest};
use cloudit_core::protocol::{CommitResponse, ScopeRole, SnapshotInfo, SnapshotKind};
use futures::{StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use uuid::Uuid;

use super::extract::{body_bytes, parse_id};
use crate::auth::extractors::AnyUser;
use crate::db::entities::{folders as folder_entity, scopes, snapshots as snapshot_entity};
use crate::errors::ApiError;
use crate::scopes::access::{FolderAccess, SnapshotAccess};
use crate::state::AppState;
use crate::sync::chunks::{self, Adopted};
use crate::sync::snapshots::{self, NewSnapshot};
use crate::sync::{manifests, usage};
use crate::time::{rfc3339, rfc3339_opt};

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
