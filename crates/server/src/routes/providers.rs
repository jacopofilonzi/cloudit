//! `/api/v1/admin/providers*` (web session of an effective admin): identity providers.
//! OIDC providers are created (and re-checked when their issuer changes) only if
//! discovery against the issuer succeeds.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    CreateProviderRequest, ErrorCode, ProviderInfo, ProviderPreset, ProviderTestResult,
    UpdateProviderRequest,
};

use super::extract::ApiJson;
use crate::auth::extractors::AdminUser;
use crate::errors::ApiError;
use crate::idp::fields::{self, ProviderFields};
use crate::idp::presets::PRESETS;
use crate::idp::store::{self, Provider};
use crate::state::AppState;

/// `GET /admin/providers/presets`.
pub async fn presets(_admin: AdminUser) -> Json<Vec<ProviderPreset>> {
    Json(PRESETS.iter().map(|p| p.to_api()).collect())
}

/// `GET /admin/providers`: non-deleted providers in display order.
pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<ProviderInfo>>, ApiError> {
    let public_url = state.public_url().await?;
    let list = store::list(&state.db, false).await?;
    Ok(Json(
        list.iter().map(|p| p.info(public_url.as_deref())).collect(),
    ))
}

async fn info(state: &AppState, provider: &Provider) -> Result<ProviderInfo, ApiError> {
    Ok(provider.info(state.public_url().await?.as_deref()))
}

/// `POST /admin/providers` → 201.
pub async fn create(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    ApiJson(body): ApiJson<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderInfo>), ApiError> {
    let id = fields::id(&body.id)?;
    let secret = fields::client_secret(body.client_secret.expose())?;
    let position = store::next_position(&state.db).await?;
    let provider_fields = ProviderFields::from_create(&body, position)?;
    if let Some(issuer) = provider_fields.issuer(body.kind) {
        state.idp.discover(&issuer, true).await?;
    }
    let provider = store::insert(
        &state.db,
        &state.secrets,
        &id,
        body.kind,
        &provider_fields,
        secret,
    )
    .await?;
    tracing::info!(admin_id = %me.user.id, provider = %id, kind = body.kind.as_str(), "identity provider created");
    Ok((StatusCode::CREATED, Json(info(&state, &provider).await?)))
}

/// `PATCH /admin/providers/{id}`.
pub async fn update(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateProviderRequest>,
) -> Result<Json<ProviderInfo>, ApiError> {
    let current = store::find(&state.db, &id).await?;
    let new_fields = current.fields.with_update(current.kind, &body)?;
    let secret = body
        .client_secret
        .as_ref()
        .map(|s| fields::client_secret(s.expose()))
        .transpose()?;
    if let Some(issuer) = new_fields.issuer(current.kind)
        && Some(&issuer) != current.issuer().as_ref()
    {
        state.idp.discover(&issuer, true).await?;
    }
    let provider = store::update(&state.db, &state.secrets, &id, &new_fields, secret).await?;
    tracing::info!(admin_id = %me.user.id, provider = %id, "identity provider updated");
    Ok(Json(info(&state, &provider).await?))
}

/// `DELETE /admin/providers/{id}` → 204: soft delete (disabled, hidden, id kept).
pub async fn delete(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !store::soft_delete(&state.db, &id).await? {
        return Err(ApiError::ProviderNotFound);
    }
    tracing::info!(admin_id = %me.user.id, provider = %id, "identity provider deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/providers/{id}/test`: fresh discovery (OIDC) and secret decryption;
/// changes nothing.
pub async fn test(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Result<Json<ProviderTestResult>, ApiError> {
    let provider = store::find(&state.db, &id).await?;
    let code = if let Err(err) = provider.client_secret(&state.secrets) {
        tracing::warn!(provider = %id, error = %format!("{err:#}"), "provider test failed");
        Some(ErrorCode::ProviderError)
    } else if let Some(issuer) = provider.issuer() {
        state
            .idp
            .discover(&issuer, true)
            .await
            .err()
            .map(|e| e.code())
    } else {
        None
    };
    Ok(Json(ProviderTestResult {
        ok: code.is_none(),
        code,
    }))
}
