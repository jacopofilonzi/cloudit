//! `/api/v1/admin/*` (web session of an effective admin): users (flags, approval),
//! password resets, invites, settings (registration, sign-up approval, auto-link,
//! public URL, retention defaults, trash retention, default quota), summary and the
//! update check ("Check now"). User
//! entries carry their storage usage and quota.

use std::time::Duration;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    AdminSummary, AdminUser as AdminUserInfo, AdminUserList, AdminUserUpdate, ApprovalStatus,
    CreateInviteRequest, InviteInfo, InviteLink, JobName, PasswordResetLink, Patch, Sensitive,
    ServerSettings, TokenTtl, UpdateSettingsRequest,
};
use serde::Deserialize;

use super::extract::{ApiJson, Storage, admin_user, invite_info, parse_id};
use crate::accounts::users::QuotaUpdate;
use crate::accounts::{admin, invites, password_resets, settings, users};
use crate::auth::extractors::AdminUser;
use crate::db::entities::users as user_entity;
use crate::errors::ApiError;
use crate::jobs::{self, store, update_check};
use crate::oauth::identities;
use crate::state::AppState;
use crate::sync::usage;
use crate::time::rfc3339;
use crate::validate;

/// Default and maximum page size of `GET /admin/users`.
const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;
/// Longest trash retention, in days (10 years).
const MAX_TRASH_RETENTION_DAYS: u32 = 3650;

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
    let used = usage::used_by_many(&state.db, &ids).await?;
    let default_quota = settings::default_quota_bytes(&state.db).await?;
    Ok(Json(AdminUserList {
        users: list
            .iter()
            .map(|u| {
                let storage = Storage {
                    used_bytes: used.get(&u.id).copied().unwrap_or(0),
                    effective_quota: usage::quota_with_default(u, default_quota),
                };
                admin_user(u, providers.remove(&u.id).unwrap_or_default(), storage)
            })
            .collect(),
        total,
    }))
}

/// The quota change a `PATCH /admin/users/{id}` asks for, validated before any write.
fn quota_change(body: &AdminUserUpdate) -> Result<QuotaUpdate, ApiError> {
    let bytes = match body.quota_bytes {
        Patch::Missing => None,
        Patch::Null => Some(None),
        Patch::Value(v) => {
            Some(Some(i64::try_from(v).map_err(|_| {
                ApiError::validation("quota_bytes", "too large")
            })?))
        }
    };
    Ok(QuotaUpdate {
        bytes,
        unlimited: body.quota_unlimited,
    })
}

/// `PATCH /admin/users/{id}` `{admin?, disabled?, quota_bytes?, quota_unlimited?}`;
/// `last_admin` if it would leave no enabled admin. `quota_bytes: null` = the server
/// default; `quota_unlimited: true` = no quota; each changes only its own column
/// (rules in `AdminUserUpdate`).
pub async fn update_user(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AdminUserUpdate>,
) -> Result<Json<AdminUserInfo>, ApiError> {
    let id = parse_id(&id)?;
    let change = quota_change(&body)?;
    let mut user = if body.admin.is_some() || body.disabled.is_some() {
        let user = admin::update_flags(&state.db, id, body.admin, body.disabled).await?;
        tracing::info!(
            admin_id = %me.user.id,
            user_id = %user.id,
            admin = user.admin,
            disabled = user.is_disabled(),
            "account flags changed"
        );
        user
    } else {
        users::find(&state.db, id)
            .await?
            .ok_or(ApiError::NotFound)?
    };
    if !change.is_empty() {
        users::set_quota(&state.db, id, change).await?;
        tracing::info!(admin_id = %me.user.id, user_id = %id, quota = ?change, "storage quota changed");
        change.apply(&mut user);
    }
    Ok(Json(user_view(&state, &user).await?))
}

async fn user_view(state: &AppState, user: &user_entity::Model) -> Result<AdminUserInfo, ApiError> {
    let mut providers = identities::provider_ids_by_user(&state.db, &[user.id]).await?;
    let storage = Storage {
        used_bytes: usage::used_by(&state.db, user.id).await?,
        effective_quota: usage::effective_quota(&state.db, user).await?,
    };
    Ok(admin_user(
        user,
        providers.remove(&user.id).unwrap_or_default(),
        storage,
    ))
}

/// `POST /admin/users/{id}/approve`: pending → approved, or rejected → approved
/// (lifts the ban). An approved account is answered unchanged.
pub async fn approve_user(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<AdminUserInfo>, ApiError> {
    let id = parse_id(&id)?;
    let decision = admin::approve(&state.db, id, me.user.id).await?;
    if decision.changed {
        tracing::info!(admin_id = %me.user.id, user_id = %id, "account approved");
    }
    Ok(Json(user_view(&state, &decision.user).await?))
}

/// `POST /admin/users/{id}/reject`: pending or approved → rejected (a ban: no sign-in
/// by any method, web sessions deleted, device tokens refused). `forbidden` for the
/// caller's own account, `last_admin` if no enabled approved admin would remain. A
/// rejected account is answered unchanged.
pub async fn reject_user(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<AdminUserInfo>, ApiError> {
    let id = parse_id(&id)?;
    let decision = admin::reject(&state.db, id, me.user.id).await?;
    if decision.changed {
        tracing::info!(admin_id = %me.user.id, user_id = %id, "account rejected");
    }
    Ok(Json(user_view(&state, &decision.user).await?))
}

/// `GET /admin/summary`: counters for the admin UI. The admin shell reads it when the
/// web app opens (and every minute), so it also requests an update check when the last
/// one started more than [`update_check::AUTO_CHECK_GAP`] ago (database-coordinated:
/// once per gap for the whole server); the answer doesn't wait for it
/// (`update_check.pending` tells the UI to read the summary again shortly).
pub async fn summary(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<AdminSummary>, ApiError> {
    if state.config.update_check {
        jobs::request_unless_recent(&state, JobName::UpdateCheck, update_check::AUTO_CHECK_GAP)
            .await?;
    }
    Ok(Json(summary_view(&state).await?))
}

/// `POST /admin/update-check` → 202 `AdminSummary`: "Check now". Refused with 429
/// `rate_limited` when a check started less than [`update_check::MANUAL_CHECK_GAP`]
/// ago (server-wide); a check already requested counts as accepted. With
/// `UPDATE_CHECK=false` nothing is requested (`update_check.enabled` is false).
pub async fn check_updates(
    State(state): State<AppState>,
    AdminUser(me): AdminUser,
) -> Result<(StatusCode, Json<AdminSummary>), ApiError> {
    if state.config.update_check {
        request_update_check(&state).await?;
        tracing::info!(admin_id = %me.user.id, "update check requested");
    }
    Ok((StatusCode::ACCEPTED, Json(summary_view(&state).await?)))
}

/// Requests an `update_check` run now, at most once per
/// [`update_check::MANUAL_CHECK_GAP`] for the whole server (also used by `POST
/// /admin/jobs/update_check/run`).
pub async fn request_update_check(state: &AppState) -> Result<(), ApiError> {
    let gap = update_check::MANUAL_CHECK_GAP;
    if jobs::request_unless_recent(state, JobName::UpdateCheck, gap).await? {
        return Ok(());
    }
    // Not requested by this call: either a run is already requested (fine: it comes),
    // or the last one started too recently.
    let row = store::get(&state.db, JobName::UpdateCheck).await?;
    let Some(row) = row.filter(|r| r.requested_at.is_none()) else {
        return Ok(());
    };
    let since = row
        .last_started_at
        .map_or(0, |at| (crate::time::now() - at).num_seconds());
    let gap_secs = i64::try_from(gap.as_secs()).unwrap_or(i64::MAX);
    let wait = u64::try_from(gap_secs.saturating_sub(since)).unwrap_or(0);
    Err(ApiError::RateLimited {
        retry_after: Duration::from_secs(wait.max(1)),
    })
}

async fn summary_view(state: &AppState) -> Result<AdminSummary, ApiError> {
    let server_version = env!("CARGO_PKG_VERSION").to_owned();
    let latest_release = settings::latest_release(&state.db).await?;
    let update_available = latest_release
        .as_ref()
        .is_some_and(|r| cloudit_core::version::is_newer(&r.version, &server_version));
    Ok(AdminSummary {
        pending_users: users::count_pending(&state.db).await?,
        server_version,
        latest_release,
        update_available,
        update_check: update_check::status(state).await?,
    })
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
    let secs = ttl
        .as_secs()
        .ok_or_else(|| ApiError::validation("expires_in", "invites always expire"))?;
    let (token, invite) =
        invites::create(&state.db, me.user.id, email, Duration::from_secs(secs)).await?;
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
    let retention = settings::retention_defaults(&state.db).await?;
    Ok(Json(ServerSettings {
        registration_mode: settings::registration_mode(&state.db).await?,
        password_new_accounts: settings::password_new_accounts(&state.db).await?,
        autolink_email: settings::autolink_email(&state.db).await?,
        public_url: settings::public_url(&state.db).await?,
        public_url_from_env: state.config.public_url.is_some(),
        effective_public_url: state.public_url().await?,
        retention_max_versions: retention.max_versions,
        retention_max_per_day: retention.max_per_day,
        trash_retention_days: settings::trash_retention_days(&state.db).await?,
        default_quota_bytes: settings::default_quota_bytes(&state.db).await?,
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
    let max_versions = body
        .retention_max_versions
        .map(|v| validate::range(v, "retention_max_versions", 0, validate::MAX_RETENTION))
        .transpose()?;
    let max_per_day = body
        .retention_max_per_day
        .map(|v| validate::range(v, "retention_max_per_day", 0, validate::MAX_RETENTION))
        .transpose()?;
    let trash_days = body
        .trash_retention_days
        .map(|v| validate::range(v, "trash_retention_days", 1, MAX_TRASH_RETENTION_DAYS))
        .transpose()?;
    if let Some(value) = body.default_quota_bytes.into_option() {
        if value.is_some_and(|v| i64::try_from(v).is_err()) {
            return Err(ApiError::validation("default_quota_bytes", "too large"));
        }
        settings::set_default_quota_bytes(&state.db, value, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, quota = ?value, "default storage quota changed");
    }
    if let Some(value) = max_versions {
        settings::set_retention_max_versions(&state.db, value, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, value, "default retention_max_versions changed");
    }
    if let Some(value) = max_per_day {
        settings::set_retention_max_per_day(&state.db, value, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, value, "default retention_max_per_day changed");
    }
    if let Some(days) = trash_days {
        settings::set_trash_retention_days(&state.db, days, Some(me.user.id)).await?;
        tracing::info!(admin_id = %me.user.id, days, "trash retention changed");
    }
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
