//! `GET /api/v1/trash` (web session): what the caller can restore.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{TrashItem, TrashItemKind};

use crate::accounts::settings;
use crate::auth::extractors::SessionUser;
use crate::errors::ApiError;
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
