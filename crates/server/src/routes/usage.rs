//! `/api/v1/me/usage`: the caller's storage usage and quota (session or device token).

use anyhow::Context as _;
use axum::Json;
use axum::extract::State;
use cloudit_core::protocol::{ScopeUsage, UsageInfo};
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _, QueryOrder as _};

use crate::auth::extractors::AnyUser;
use crate::db::entities::scopes;
use crate::errors::ApiError;
use crate::state::AppState;
use crate::sync::usage;

/// `GET /me/usage`: the scopes the caller owns (trashed included) with their stored
/// bytes, the total and the effective quota.
pub async fn get(State(state): State<AppState>, me: AnyUser) -> Result<Json<UsageInfo>, ApiError> {
    let owned = scopes::Entity::find()
        .filter(scopes::Column::OwnerId.eq(me.user.id))
        .order_by_asc(scopes::Column::NameKey)
        .order_by_asc(scopes::Column::Id)
        .all(&state.db)
        .await
        .context("listing owned scopes")?;
    let scopes: Vec<ScopeUsage> = owned
        .iter()
        .map(|s| ScopeUsage {
            id: s.id.to_string(),
            name: s.name.clone(),
            stored_bytes: usage::bytes(s.stored_bytes),
            trashed: s.deleted_at.is_some(),
        })
        .collect();
    Ok(Json(UsageInfo {
        used_bytes: scopes
            .iter()
            .map(|s| s.stored_bytes)
            .fold(0, u64::saturating_add),
        quota_bytes: usage::effective_quota(&state.db, &me.user).await?,
        scopes,
    }))
}
