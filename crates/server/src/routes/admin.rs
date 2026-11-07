//! `/api/v1/admin/*` (web session of an effective admin): users, password resets,
//! invites, settings.

use std::time::Duration;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    AdminUser as AdminUserInfo, AdminUserList, AdminUserUpdate, ApprovalStatus,
    CreateInviteRequest, InviteInfo, InviteLink, PasswordResetLink, Sensitive, ServerSettings,
    TokenTtl, UpdateSettingsRequest,
};
use serde::Deserialize;

use super::extract::{ApiJson, admin_user, invite_info, parse_id};
use crate::accounts::{admin, invites, password_resets, settings, users};
use crate::auth::extractors::AdminUser;
use crate::errors::ApiError;
use crate::oauth::identities;
use crate::state::AppState;
use crate::time::rfc3339;
use crate::validate;

/// Default and maximum page size of `GET /admin/users`.
const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    limit: Option<u64>,
    offset: Option<u64>,
    approval_status: Option<ApprovalStatus>,
}

/// `GET /admin/users?limit=&offset=&approval_status=`: oldest first.
pub async fn list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Json<AdminUserList>, ApiError> {
    let Query(params) = params.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::validation("limit", format!("1 to {MAX_LIMIT}")));
    }
    let (list, total) = users::list(
        &state.db,
        params.approval_status,
        limit,
        params.offset.unwrap_or(0),
    )
    .await?;
    let ids: Vec<_> = list.iter().map(|u| u.id).collect();
    let mut providers = identities::provider_ids_by_user(&state.db, &ids).await?;
    Ok(Json(AdminUserList {
        users: list
            .iter()
            .map(|u| admin_user(u, providers.remove(&u.id).unwrap_or_default()))
            .collect(),
        total,
    }))
}

/// `PATCH /admin/users/{id}` `{admin?, disabled?}`; `last_admin` if it would leave no
/// enabled admin.
pub async fn update_user(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AdminUserUpdate>,
) -> Result<Json<AdminUserInfo>, ApiError> {
    let id = parse_id(&id)?;
    let user = admin::update_flags(&state.db, id, body.admin, body.disabled).await?;
    tracing::info!(
        admin_id = %me.user.id,
        user_id = %user.id,
        admin = user.admin,
        disabled = user.is_disabled(),
        "account flags changed"
    );
    let mut providers = identities::provider_ids_by_user(&state.db, &[user.id]).await?;
    Ok(Json(admin_user(
        &user,
        providers.remove(&user.id).unwrap_or_default(),
    )))
}

/// `POST /admin/users/{id}/password-reset` → 201 with a 24-hour single-use link. Works
/// for accounts without a password too (the reset sets one).
pub async fn create_password_reset(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PasswordResetLink>), ApiError> {
    let id = parse_id(&id)?;
    let user = users::find(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (token, reset) = password_resets::create(&state.db, user.id, Some(me.user.id)).await?;
    tracing::info!(admin_id = %me.user.id, user_id = %user.id, "password reset created");
    Ok((
        StatusCode::CREATED,
        Json(PasswordResetLink {
            path: format!("/reset-password/{}", token.expose()),
            token: Sensitive::new(token.expose()),
            expires_at: rfc3339(reset.expires_at),
        }),
    ))
}

/// `GET /admin/invites`: newest first.
pub async fn list_invites(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<InviteInfo>>, ApiError> {
    let now = crate::time::now();
    let list = invites::list(&state.db).await?;
    Ok(Json(list.iter().map(|i| invite_info(i, now)).collect()))
}

/// `POST /admin/invites` `{email?, expires_in?}` (default 7 days) → 201.
pub async fn create_invite(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    ApiJson(body): ApiJson<CreateInviteRequest>,
) -> Result<(StatusCode, Json<InviteLink>), ApiError> {
    let email = body
        .email
        .as_deref()
        .map(|e| validate::email(e, "email"))
        .transpose()?;
    let ttl = body.expires_in.unwrap_or(TokenTtl::SevenDays);
    let (token, invite) = invites::create(
        &state.db,
        me.user.id,
        email,
        Duration::from_secs(ttl.as_secs()),
    )
    .await?;
    tracing::info!(admin_id = %me.user.id, invite_id = %invite.id, "invite created");
    Ok((
        StatusCode::CREATED,
        Json(InviteLink {
            id: invite.id.to_string(),
            path: format!("/invite/{}", token.expose()),
            token: Sensitive::new(token.expose()),
            expires_at: rfc3339(invite.expires_at),
        }),
    ))
}

/// `DELETE /admin/invites/{id}` → 204 (revokes; soft).
pub async fn revoke_invite(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_id(&id)?;
    invites::revoke(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /admin/settings`.
pub async fn get_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<ServerSettings>, ApiError> {
    Ok(Json(ServerSettings {
        registration_mode: settings::registration_mode(&state.db).await?,
        password_new_accounts: settings::password_new_accounts(&state.db).await?,
        autolink_email: settings::autolink_email(&state.db).await?,
        public_url: settings::public_url(&state.db).await?,
        public_url_from_env: state.config.public_url.is_some(),
        effective_public_url: state.public_url().await?,
    }))
}

/// `PATCH /admin/settings`: every field is validated before anything is written.
pub async fn update_settings(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    ApiJson(body): ApiJson<UpdateSettingsRequest>,
) -> Result<Json<ServerSettings>, ApiError> {
    let public_url = body
        .public_url
        .as_deref()
        .map(|raw| match raw.trim() {
            "" => Ok(None),
            url => validate::public_url(url, "public_url").map(Some),
        })
        .transpose()?;
    if let Some(url) = public_url {
        settings::set_public_url(&state.db, url.as_deref(), Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, public_url = url.as_deref().unwrap_or("-"), "public URL changed");
    }
    if let Some(on) = body.autolink_email {
        settings::set_autolink_email(&state.db, on, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, on, "email auto-link changed");
    }
    if let Some(policy) = body.password_new_accounts {
        settings::set_password_new_accounts(&state.db, policy, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, policy = policy.as_str(), "password sign-up approval changed");
    }
    if let Some(mode) = body.registration_mode {
        settings::set_registration_mode(&state.db, mode, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, ?mode, "registration mode changed");
    }
    get_settings(State(state), AdminUser(me)).await
}
