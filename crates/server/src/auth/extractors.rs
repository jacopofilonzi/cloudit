//! Extractors stating what each route accepts (the [`super::middleware`] resolved the
//! credentials):
//!
//! - [`SessionUser`]: a web session only. Device tokens get `forbidden` (403): devices
//!   can't manage devices, sessions, admin or their own credentials.
//! - [`AnyUser`]: a web session or a device token (e.g. `GET /me`, future sync routes).
//! - [`AdminUser`]: a web session of an effective admin, else `forbidden`.
//! - [`Client`]: the request's [`ClientInfo`].
//!
//! No credentials (or invalid / expired ones) give `unauthorized` (401); a disabled
//! account gives `account_disabled` (403) on every authenticated route.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use super::client::ClientInfo;
use super::middleware::Auth;
use crate::db::entities::users;
use crate::errors::ApiError;

fn auth(parts: &Parts) -> &Auth {
    static ANONYMOUS: Auth = Auth::Anonymous;
    parts.extensions.get::<Auth>().unwrap_or(&ANONYMOUS)
}

fn enabled(user: &users::Model) -> Result<users::Model, ApiError> {
    if user.is_disabled() {
        Err(ApiError::AccountDisabled)
    } else {
        Ok(user.clone())
    }
}

/// A user authenticated by a web session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user: users::Model,
    pub session_id: Uuid,
}

impl<S: Send + Sync> FromRequestParts<S> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        match auth(parts) {
            Auth::Session { user, session } => Ok(Self {
                user: enabled(user)?,
                session_id: session.id,
            }),
            Auth::Device { user, .. } => {
                enabled(user)?;
                Err(ApiError::Forbidden)
            }
            Auth::Anonymous => Err(ApiError::Unauthorized),
        }
    }
}

/// A user authenticated by a web session or a device token.
#[derive(Debug, Clone)]
pub struct AnyUser {
    pub user: users::Model,
    /// Set for a web session.
    pub session_id: Option<Uuid>,
    /// Set for a device token.
    pub device_id: Option<Uuid>,
}

impl<S: Send + Sync> FromRequestParts<S> for AnyUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        match auth(parts) {
            Auth::Session { user, session } => Ok(Self {
                user: enabled(user)?,
                session_id: Some(session.id),
                device_id: None,
            }),
            Auth::Device { user, device } => Ok(Self {
                user: enabled(user)?,
                session_id: None,
                device_id: Some(device.id),
            }),
            Auth::Anonymous => Err(ApiError::Unauthorized),
        }
    }
}

/// An effective admin authenticated by a web session.
#[derive(Debug, Clone)]
pub struct AdminUser(pub SessionUser);

impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = SessionUser::from_request_parts(parts, state).await?;
        if session.user.is_admin() {
            Ok(Self(session))
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

/// The request's [`ClientInfo`].
#[derive(Debug, Clone)]
pub struct Client(pub ClientInfo);

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<ClientInfo>()
            .cloned()
            .map(Self)
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("auth middleware not installed")))
    }
}
