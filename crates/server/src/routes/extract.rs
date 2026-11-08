//! Request extractors that answer with [`ApiError`] JSON instead of axum's plain-text
//! rejections, and model → API conversions shared by the route modules.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{AdminUser, DeviceInfo, InviteInfo, SessionInfo, UserProfile};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::accounts::invites;
use crate::db::entities::{devices, invites as invite_entity, sessions, users};
use crate::errors::ApiError;
use crate::time::{rfc3339, rfc3339_opt};

/// JSON body; malformed JSON, a wrong content type or unknown fields give
/// `bad_request` (400), a body over the route's limit `payload_too_large` (413).
pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ApiError::PayloadTooLarge)
            }
            Err(rejection) => Err(ApiError::BadRequest(rejection.body_text())),
        }
    }
}

/// A raw body (take it as `Result<Bytes, BytesRejection>`): over the route's
/// `DefaultBodyLimit` → `payload_too_large` (413), other failures `bad_request`.
pub fn body_bytes(body: Result<Bytes, BytesRejection>) -> Result<Bytes, ApiError> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::PayloadTooLarge
        } else {
            ApiError::BadRequest(rejection.body_text())
        }
    })
}

/// An id from the path; anything that isn't a UUID is `not_found`.
pub fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.parse().map_err(|_| ApiError::NotFound)
}

pub fn profile(user: &users::Model) -> UserProfile {
    UserProfile {
        id: user.id.to_string(),
        email: user.email.clone(),
        email_verified: user.email_verified,
        display_name: user.display_name.clone(),
        is_admin: user.is_admin(),
        has_password: user.password_hash.is_some(),
        created_at: rfc3339(user.created_at),
    }
}

/// Storage figures of an account in [`admin_user`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Storage {
    pub used_bytes: u64,
    /// The effective quota (own, else the server default).
    pub effective_quota: Option<u64>,
}

/// `identity_providers`: provider ids of the user's identities (sorted).
pub fn admin_user(
    user: &users::Model,
    identity_providers: Vec<String>,
    storage: Storage,
) -> AdminUser {
    AdminUser {
        id: user.id.to_string(),
        email: user.email.clone(),
        email_verified: user.email_verified,
        display_name: user.display_name.clone(),
        is_admin: user.is_admin(),
        admin: user.admin,
        admin_from_groups: user.admin_from_groups,
        disabled_at: rfc3339_opt(user.disabled_at),
        has_password: user.password_hash.is_some(),
        identity_providers,
        approval_status: user.approval(),
        signup_source: user.signup_source.clone(),
        approval_decided_at: rfc3339_opt(user.approval_decided_at),
        approval_decided_by: user.approval_decided_by.map(|id| id.to_string()),
        created_at: rfc3339(user.created_at),
        used_bytes: storage.used_bytes,
        quota_bytes: user.quota_bytes.map(crate::sync::usage::bytes),
        effective_quota_bytes: storage.effective_quota,
    }
}

pub fn session_info(session: &sessions::Model, current: Uuid) -> SessionInfo {
    SessionInfo {
        id: session.id.to_string(),
        created_at: rfc3339(session.created_at),
        last_seen_at: rfc3339(session.last_seen_at),
        expires_at: rfc3339(session.expires_at),
        user_agent: session.user_agent.clone(),
        current: session.id == current,
    }
}

pub fn device_info(device: &devices::Model) -> DeviceInfo {
    DeviceInfo {
        id: device.id.to_string(),
        name: device.name.clone(),
        os: device.os.clone(),
        created_at: rfc3339(device.created_at),
        last_seen_at: rfc3339_opt(device.last_seen_at),
        expires_at: rfc3339_opt(device.expires_at),
        revoked_at: rfc3339_opt(device.revoked_at),
    }
}

pub fn invite_info(invite: &invite_entity::Model, now: DateTime<Utc>) -> InviteInfo {
    InviteInfo {
        id: invite.id.to_string(),
        email: invite.email.clone(),
        status: invites::status(invite, now),
        created_by: invite.created_by.to_string(),
        created_at: rfc3339(invite.created_at),
        expires_at: rfc3339(invite.expires_at),
        used_at: rfc3339_opt(invite.used_at),
        used_by: invite.used_by.map(|id| id.to_string()),
        revoked_at: rfc3339_opt(invite.revoked_at),
    }
}

/// An optional JSON body: empty (or only whitespace) gives `T::default()`; anything
/// else must parse, else `bad_request`.
pub fn optional_json<T: DeserializeOwned + Default>(body: &[u8]) -> Result<T, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(err.to_string()))
}
