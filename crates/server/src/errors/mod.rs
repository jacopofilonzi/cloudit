//! API error type: every handler returns `Result<_, ApiError>`, and every error
//! leaves the server as JSON `{ "code": "...", "message": "..." }` (see
//! [`cloudit_core::protocol::ApiErrorBody`]), plus `field` for validation errors and a
//! `Retry-After` header for rate limits.
//!
//! `code` is stable and machine-readable; `message` is English for logs. Internal
//! errors are logged with their full context but answered with a generic message,
//! so causes (paths, SQL, upstream bodies) never leak to clients.

use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cloudit_core::protocol::{ApiErrorBody, ErrorCode};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("service unavailable: {0}")]
    Unavailable(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("account disabled")]
    AccountDisabled,
    #[error("registration is closed")]
    RegistrationClosed,
    #[error("invalid invite")]
    InviteInvalid,
    #[error("invalid or expired token")]
    TokenInvalid,
    #[error("email already in use")]
    EmailTaken,
    #[error("invalid {field}: {message}")]
    Validation {
        field: &'static str,
        message: String,
    },
    #[error("too many requests")]
    RateLimited { retry_after: Duration },
    #[error("cross-site request refused")]
    CsrfFailed,
    #[error("first-run setup required")]
    SetupRequired,
    #[error("setup already completed")]
    SetupAlreadyDone,
    #[error("invalid setup token")]
    SetupTokenInvalid,
    #[error("this would leave no enabled admin")]
    LastAdmin,
    #[error("identity provider not found")]
    ProviderNotFound,
    #[error("provider id already used")]
    ProviderIdTaken,
    #[error("identity provider discovery failed")]
    ProviderDiscoveryFailed,
    #[error("identity provider error")]
    ProviderError,
    #[error("the server's public URL is not configured")]
    PublicUrlRequired,
    #[error("identity provider disabled")]
    ProviderDisabled,
    #[error("invalid, expired or used sign-in state")]
    OauthStateInvalid,
    #[error("sign-in cancelled at the provider")]
    OauthCancelled,
    #[error("not in an allowed group")]
    GroupNotAllowed,
    #[error("identity linked to another account")]
    IdentityInUse,
    #[error("identity not linked to an account")]
    IdentityNotLinked,
    #[error("pending sign-up missing or expired")]
    OnboardingExpired,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    /// A `validation_failed` error on `field`.
    pub fn validation(field: &'static str, message: impl Into<String>) -> Self {
        Self::Validation {
            field,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadRequest(_) => ErrorCode::BadRequest,
            Self::NotFound => ErrorCode::NotFound,
            Self::Unavailable(_) => ErrorCode::Unavailable,
            Self::Unauthorized => ErrorCode::Unauthorized,
            Self::Forbidden => ErrorCode::Forbidden,
            Self::InvalidCredentials => ErrorCode::InvalidCredentials,
            Self::AccountDisabled => ErrorCode::AccountDisabled,
            Self::RegistrationClosed => ErrorCode::RegistrationClosed,
            Self::InviteInvalid => ErrorCode::InviteInvalid,
            Self::TokenInvalid => ErrorCode::TokenInvalid,
            Self::EmailTaken => ErrorCode::EmailTaken,
            Self::Validation { .. } => ErrorCode::ValidationFailed,
            Self::RateLimited { .. } => ErrorCode::RateLimited,
            Self::CsrfFailed => ErrorCode::CsrfFailed,
            Self::SetupRequired => ErrorCode::SetupRequired,
            Self::SetupAlreadyDone => ErrorCode::SetupAlreadyDone,
            Self::SetupTokenInvalid => ErrorCode::SetupTokenInvalid,
            Self::LastAdmin => ErrorCode::LastAdmin,
            Self::ProviderNotFound => ErrorCode::ProviderNotFound,
            Self::ProviderIdTaken => ErrorCode::ProviderIdTaken,
            Self::ProviderDiscoveryFailed => ErrorCode::ProviderDiscoveryFailed,
            Self::ProviderError => ErrorCode::ProviderError,
            Self::PublicUrlRequired => ErrorCode::PublicUrlRequired,
            Self::ProviderDisabled => ErrorCode::ProviderDisabled,
            Self::OauthStateInvalid => ErrorCode::OauthStateInvalid,
            Self::OauthCancelled => ErrorCode::OauthCancelled,
            Self::GroupNotAllowed => ErrorCode::GroupNotAllowed,
            Self::IdentityInUse => ErrorCode::IdentityInUse,
            Self::IdentityNotLinked => ErrorCode::IdentityNotLinked,
            Self::OnboardingExpired => ErrorCode::OnboardingExpired,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_)
            | Self::InviteInvalid
            | Self::TokenInvalid
            | Self::OauthStateInvalid
            | Self::OauthCancelled => StatusCode::BAD_REQUEST,
            Self::NotFound | Self::ProviderNotFound => StatusCode::NOT_FOUND,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Unauthorized | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Forbidden
            | Self::AccountDisabled
            | Self::RegistrationClosed
            | Self::CsrfFailed
            | Self::SetupTokenInvalid
            | Self::ProviderDisabled
            | Self::GroupNotAllowed => StatusCode::FORBIDDEN,
            Self::EmailTaken
            | Self::SetupRequired
            | Self::SetupAlreadyDone
            | Self::LastAdmin
            | Self::ProviderIdTaken
            | Self::PublicUrlRequired
            | Self::IdentityInUse
            | Self::IdentityNotLinked => StatusCode::CONFLICT,
            Self::OnboardingExpired => StatusCode::GONE,
            Self::ProviderDiscoveryFailed | Self::ProviderError => StatusCode::BAD_GATEWAY,
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = match &self {
            Self::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                "internal server error".to_owned()
            }
            other => other.to_string(),
        };
        let field = match &self {
            Self::Validation { field, .. } => Some((*field).to_owned()),
            _ => None,
        };
        let body = ApiErrorBody {
            code: self.code(),
            message,
            field,
        };
        let mut response = (self.status(), Json(body)).into_response();
        if let Self::RateLimited { retry_after } = self {
            // Whole seconds, rounded up, at least 1.
            let secs = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs.max(1)));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_of(err: ApiError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn not_found_maps_to_404_with_code() {
        let (status, body) = body_of(ApiError::NotFound).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "not_found");
        assert!(body.get("field").is_none());
    }

    #[tokio::test]
    async fn internal_hides_the_cause() {
        let err = anyhow::anyhow!("secret path /var/lib/x").context("reading chunk");
        let (status, body) = body_of(ApiError::Internal(err)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        assert_eq!(body["message"], "internal server error");
    }

    #[tokio::test]
    async fn validation_names_the_field() {
        let (status, body) = body_of(ApiError::validation("email", "not an email")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(body["field"], "email");
    }

    #[tokio::test]
    async fn rate_limited_sets_retry_after() {
        let response = ApiError::RateLimited {
            retry_after: Duration::from_millis(1500),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
    }
}
