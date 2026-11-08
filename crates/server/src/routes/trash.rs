//! `/api/v1/trash*` (web session): what the caller can restore, and deleting it for
//! good (marks the items; the `trash_purge` job does the work).

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{JobName, TrashEmptyResponse, TrashItem, TrashItemKind};

use super::extract::parse_id;
use crate::accounts::settings;
use crate::auth::extractors::SessionUser;
use crate::errors::ApiError;
use crate::jobs;
use crate::scopes::access::{FolderAccess, ScopeAccess};
use crate::scopes::{folders, store};
use crate::state::AppState;
use crate::time::rfc3339;

/// Scopes the caller owns in the trash, and trashed folders of scopes (outside the
/// trash) where the caller is owner or admin; most recently deleted first. Folders of
/// a trashed scope come back with it, so they aren't listed on their own.
pub async fn list(
    State(state): State<AppState>,
    me: SessionUser,
) -> Result<Json<Vec<TrashItem>>, ApiError> {
    let days = settings::trash_retention_days(&state.db).await?;
    let keep = Duration::from_secs(u64::from(days) * 86_400);
    let purge_at = |deleted: DateTime<Utc>| -> Result<String, ApiError> {
        Ok(rfc3339(crate::time::add(deleted, keep)?))
    };
    let mut items = Vec::new();
    for scope in store::trashed_owned(&state.db, me.user.id).await? {
        let Some(deleted_at) = scope.deleted_at else {
            continue;
        };
        items.push((
            deleted_at,
            TrashItem {
                kind: TrashItemKind::Scope,
                id: scope.id.to_string(),
                name: scope.name.clone(),
                scope_id: scope.id.to_string(),
                scope_name: scope.name.clone(),
                deleted_at: rfc3339(deleted_at),
                deleted_by: scope.deleted_by.map(|id| id.to_string()),
                purge_at: purge_at(deleted_at)?,
            },
        ));
    }
    let managed = store::managed(&state.db, me.user.id).await?;
    let ids: Vec<_> = managed.iter().map(|s| s.id).collect();
    for folder in folders::trashed_in(&state.db, &ids).await? {
        let (Some(deleted_at), Some(scope)) = (
            folder.deleted_at,
            managed.iter().find(|s| s.id == folder.scope_id),
        ) else {
            continue;
        };
        items.push((
            deleted_at,
            TrashItem {
                kind: TrashItemKind::Folder,
                id: folder.id.to_string(),
                name: folder.name.clone(),
                scope_id: scope.id.to_string(),
                scope_name: scope.name.clone(),
                deleted_at: rfc3339(deleted_at),
                deleted_by: folder.deleted_by.map(|id| id.to_string()),
                purge_at: purge_at(deleted_at)?,
            },
        ));
    }
    items.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.id.cmp(&y.id)));
    Ok(Json(items.into_iter().map(|(_, item)| item).collect()))
}

/// `DELETE /trash/scopes/{id}` (owner, session) → 202: marks a trashed scope for
/// deletion for good; it disappears from the trash at once and the `trash_purge` job
/// deletes it (with all its folders, versions and stored data) shortly after.
pub async fn purge_scope(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let access = ScopeAccess::load_trashed(&state.db, me.user.id, parse_id(&id)?).await?;
    if !store::request_purge(&state.db, access.scope.id).await? {
        return Err(ApiError::NotFound);
    }
    jobs::request(&state, JobName::TrashPurge).await?;
    tracing::info!(user_id = %me.user.id, scope_id = %access.scope.id, "scope marked for permanent deletion");
    Ok(StatusCode::ACCEPTED)
}

/// `DELETE /trash/folders/{id}` (owner or admin of its scope, session) → 202: like
/// [`purge_scope`] for a trashed folder.
pub async fn purge_folder(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let access = FolderAccess::load_trashed(&state.db, me.user.id, parse_id(&id)?).await?;
    if !folders::request_purge(&state.db, access.folder.id).await? {
        return Err(ApiError::NotFound);
    }
    jobs::request(&state, JobName::TrashPurge).await?;
    tracing::info!(user_id = %me.user.id, folder_id = %access.folder.id, "folder marked for permanent deletion");
    Ok(StatusCode::ACCEPTED)
}

/// `POST /trash/empty` (session) → 202: marks everything `GET /trash` lists for the
/// caller for deletion for good.
pub async fn empty(
    State(state): State<AppState>,
    me: SessionUser,
) -> Result<(StatusCode, Json<TrashEmptyResponse>), ApiError> {
    let mut out = TrashEmptyResponse {
        scopes: 0,
        folders: 0,
    };
    for scope in store::trashed_owned(&state.db, me.user.id).await? {
        if store::request_purge(&state.db, scope.id).await? {
            out.scopes += 1;
        }
    }
    let managed = store::managed(&state.db, me.user.id).await?;
    let ids: Vec<_> = managed.iter().map(|s| s.id).collect();
    for folder in folders::trashed_in(&state.db, &ids).await? {
        if folders::request_purge(&state.db, folder.id).await? {
            out.folders += 1;
        }
    }
    if out.scopes + out.folders > 0 {
        jobs::request(&state, JobName::TrashPurge).await?;
    }
    tracing::info!(user_id = %me.user.id, scopes = out.scopes, folders = out.folders, "trash emptied");
    Ok((StatusCode::ACCEPTED, Json(out)))
}
