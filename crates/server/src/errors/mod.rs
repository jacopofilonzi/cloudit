//! API error type: every handler returns `Result<_, ApiError>`, and every error
//! leaves the server as JSON `{ "code": "...", "message": "..." }` (see
//! [`cloudit_core::protocol::ApiErrorBody`]).
//!
//! `code` is stable and machine-readable; `message` is English for logs. Internal
//! errors are logged with their full context but answered with a generic message,
//! so causes (paths, SQL, upstream bodies) never leak to clients.

use axum::Json;
use axum::http::StatusCode;
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
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadRequest(_) => ErrorCode::BadRequest,
            Self::NotFound => ErrorCode::NotFound,
            Self::Unavailable(_) => ErrorCode::Unavailable,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
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
        let body = ApiErrorBody {
            code: self.code(),
            message,
        };
        (self.status(), Json(body)).into_response()
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
    }

    #[tokio::test]
    async fn internal_hides_the_cause() {
        let err = anyhow::anyhow!("secret path /var/lib/x").context("reading chunk");
        let (status, body) = body_of(ApiError::Internal(err)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        assert_eq!(body["message"], "internal server error");
    }
}
