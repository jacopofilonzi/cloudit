//! `/api/v1/scopes/{id}/folders` and `/api/v1/folders/{id}`. Every handler resolves
//! the caller's role through `scopes::access` (404 for what it can't see, 403 for a
//! role too low).
//!
//! Device tokens (`AnyUser`) may list, read and create folders (the desktop client
//! links local folders); changing, trashing and restoring need a web session.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    CreateFolderRequest, FolderHead, FolderInfo, RestoreRequest, ScopeRole, UpdateFolderRequest,
};

use super::extract::{ApiJson, optional_json, parse_id};
use crate::accounts::settings::{self, RetentionDefaults};
use crate::auth::extractors::{AnyUser, SessionUser};
use crate::db::entities::folders as folder_entity;
use crate::errors::ApiError;
use crate::scopes::access::{FolderAccess, ScopeAccess};
use crate::scopes::folders::{self, Changes, Retention};
use crate::scopes::names;
use crate::state::AppState;
use crate::sync::snapshots::{self, FolderSync};
use crate::sync::usage;
use crate::time::rfc3339;
use crate::validate;

fn stored(value: Option<i32>) -> Option<u32> {
    value.map(|v| u32::try_from(v).unwrap_or(0))
}

pub fn folder_info(
    folder: &folder_entity::Model,
    defaults: RetentionDefaults,
    sync: Option<&FolderSync>,
) -> FolderInfo {
    let head = sync.and_then(|s| s.head.as_ref()).map(|h| FolderHead {
        snapshot_id: h.id.to_string(),
        created_at: rfc3339(h.created_at),
        logical_size: usage::bytes(h.logical_size),
        file_count: usage::bytes(h.file_count),
    });
    let max_versions = stored(folder.retention_max_versions);
    let max_per_day = stored(folder.retention_max_per_day);
    FolderInfo {
        id: folder.id.to_string(),
        scope_id: folder.scope_id.to_string(),
        name: folder.name.clone(),
        head_snapshot_id: folder.head_snapshot_id.map(|id| id.to_string()),
        head,
        unresolved_conflicts: sync.map_or(0, |s| s.unresolved_conflicts),
        retention_max_versions: max_versions,
        retention_max_per_day: max_per_day,
        effective_retention_max_versions: max_versions.unwrap_or(defaults.max_versions),
        effective_retention_max_per_day: max_per_day.unwrap_or(defaults.max_per_day),
        created_by: folder.created_by.to_string(),
        created_at: rfc3339(folder.created_at),
        updated_at: rfc3339(folder.updated_at),
    }
}

async fn view(state: &AppState, folder: &folder_entity::Model) -> Result<FolderInfo, ApiError> {
    let defaults = settings::retention_defaults(&state.db).await?;
    let sync = snapshots::folder_sync(&state.db, &[folder]).await?;
    Ok(folder_info(folder, defaults, sync.get(&folder.id)))
}

async fn reload(state: &AppState, id: uuid::Uuid) -> Result<folder_entity::Model, ApiError> {
    Ok(folders::find(&state.db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("folder vanished"))?)
}

fn retention(value: Option<u32>, field: &'static str) -> Result<Option<i32>, ApiError> {
    value.map(|v| validate::retention(v, field)).transpose()
}

/// `GET /scopes/{id}/folders` (any member): folders outside the trash, by name.
pub async fn list(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<FolderInfo>>, ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    let defaults = settings::retention_defaults(&state.db).await?;
    let list = folders::list(&state.db, access.scope.id).await?;
    let sync = snapshots::folder_sync(&state.db, &list.iter().collect::<Vec<_>>()).await?;
    Ok(Json(
        list.iter()
            .map(|f| folder_info(f, defaults, sync.get(&f.id)))
            .collect(),
    ))
}

/// `POST /scopes/{id}/folders {name, retention_max_versions?, retention_max_per_day?}`
/// (editor; a retention override needs admin) → 201.
pub async fn create(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CreateFolderRequest>,
) -> Result<(StatusCode, Json<FolderInfo>), ApiError> {
    let access = ScopeAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    if body.retention_max_versions.is_some() || body.retention_max_per_day.is_some() {
        access.require(ScopeRole::Admin)?;
    }
    let name = names::parse(&body.name, "name")?;
    let retention = Retention {
        max_versions: retention(body.retention_max_versions, "retention_max_versions")?,
        max_per_day: retention(body.retention_max_per_day, "retention_max_per_day")?,
    };
    let folder = folders::create(&state.db, access.scope.id, name, retention, me.user.id).await?;
    tracing::info!(user_id = %me.user.id, scope_id = %access.scope.id, folder_id = %folder.id, "folder created");
    Ok((StatusCode::CREATED, Json(view(&state, &folder).await?)))
}

/// `GET /folders/{id}` (any member).
pub async fn get(
    State(state): State<AppState>,
    me: AnyUser,
    Path(id): Path<String>,
) -> Result<Json<FolderInfo>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    Ok(Json(view(&state, &access.folder).await?))
}

/// `PATCH /folders/{id} {name?, retention_max_versions?, retention_max_per_day?}`:
/// renaming needs editor, retention changes admin (`null` resets to the default).
pub async fn update(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateFolderRequest>,
) -> Result<Json<FolderInfo>, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Editor)?;
    if !body.retention_max_versions.is_missing() || !body.retention_max_per_day.is_missing() {
        access.require(ScopeRole::Admin)?;
    }
    let changes = Changes {
        name: body
            .name
            .as_deref()
            .map(|n| names::parse(n, "name"))
            .transpose()?,
        max_versions: body
            .retention_max_versions
            .into_option()
            .map(|v| retention(v, "retention_max_versions"))
            .transpose()?,
        max_per_day: body
            .retention_max_per_day
            .into_option()
            .map(|v| retention(v, "retention_max_per_day"))
            .transpose()?,
    };
    folders::update(&state.db, access.folder.id, changes).await?;
    let folder = reload(&state, access.folder.id).await?;
    Ok(Json(view(&state, &folder).await?))
}

/// `DELETE /folders/{id}` (admin) → 204: moves the folder to the trash.
pub async fn delete(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let access = FolderAccess::load(&state.db, me.user.id, parse_id(&id)?).await?;
    access.require(ScopeRole::Admin)?;
    if !folders::trash(&state.db, access.folder.id, me.user.id).await? {
        return Err(ApiError::NotFound);
    }
    tracing::info!(user_id = %me.user.id, folder_id = %access.folder.id, "folder moved to the trash");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /folders/{id}/restore {name?}` (admin of the folder's scope, which must be
/// outside the trash). `name_taken` if its name is in use: retry with another `name`.
pub async fn restore(
    State(state): State<AppState>,
    me: SessionUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<FolderInfo>, ApiError> {
    let access = FolderAccess::load_trashed(&state.db, me.user.id, parse_id(&id)?).await?;
    let body: RestoreRequest = optional_json(&body)?;
    let name = body
        .name
        .as_deref()
        .map(|n| names::parse(n, "name"))
        .transpose()?;
    folders::restore(&state.db, &access.folder, name).await?;
    tracing::info!(user_id = %me.user.id, folder_id = %access.folder.id, "folder restored");
    let folder = reload(&state, access.folder.id).await?;
    Ok(Json(view(&state, &folder).await?))
}
