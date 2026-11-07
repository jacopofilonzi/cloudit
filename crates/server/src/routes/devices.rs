//! `/api/v1/devices`: desktop login (loopback + PKCE) and device management.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use cloudit_core::protocol::{
    DeviceAuthorizeRequest, DeviceAuthorizeResponse, DeviceInfo, DeviceTokenRequest,
    DeviceTokenResponse, Sensitive, UpdateDeviceRequest,
};
use sea_orm::TransactionTrait as _;

use super::extract::{ApiJson, device_info, parse_id};
use crate::accounts::users;
use crate::auth::extractors::{Client, SessionUser};
use crate::auth::{limits, tokens};
use crate::db::entities::device_auth_codes;
use crate::devices::{self, codes, redirect};
use crate::errors::ApiError;
use crate::state::AppState;
use crate::validate;

/// Longest `os` value, in characters.
const MAX_OS_CHARS: usize = 50;
/// Longest `state` value, in characters.
const MAX_STATE_CHARS: usize = 512;

/// A PKCE S256 challenge: 43 base64url characters (32 bytes, no padding).
fn validate_challenge(challenge: &str) -> Result<(), ApiError> {
    let ok = challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(ApiError::validation(
            "code_challenge",
            "expected 43 base64url characters",
        ))
    }
}

/// A PKCE verifier (RFC 7636 §4.1): 43 to 128 unreserved characters.
fn verifier_is_valid(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// `POST /devices/authorize` (session) → `{redirect_url}` with a 2-minute code.
pub async fn authorize(
    State(state): State<AppState>,
    session: SessionUser,
    ApiJson(body): ApiJson<DeviceAuthorizeRequest>,
) -> Result<Json<DeviceAuthorizeResponse>, ApiError> {
    validate_challenge(&body.code_challenge)?;
    let redirect_uri = redirect::validate(&body.redirect_uri)?;
    if body.state.is_empty()
        || body.state.chars().count() > MAX_STATE_CHARS
        || body.state.chars().any(char::is_control)
    {
        return Err(ApiError::validation(
            "state",
            format!("1 to {MAX_STATE_CHARS} characters, no control characters"),
        ));
    }
    let device_name = validate::text(&body.device_name, "device_name", validate::MAX_NAME_CHARS)?;
    let os = validate::text(&body.os, "os", MAX_OS_CHARS)?;
    let ttl = body.token_ttl.unwrap_or_default();

    let code = codes::create(
        &state.db,
        codes::NewCode {
            user_id: session.user.id,
            code_challenge: body.code_challenge,
            redirect_uri: body.redirect_uri,
            device_name,
            os,
            token_ttl: Duration::from_secs(ttl.as_secs()),
        },
    )
    .await?;
    Ok(Json(DeviceAuthorizeResponse {
        redirect_url: redirect::with_code(redirect_uri, code.expose(), &body.state),
    }))
}

/// Checks a consumed code against the exchange request.
fn check_exchange(
    code: &device_auth_codes::Model,
    body: &DeviceTokenRequest,
) -> Result<(), ApiError> {
    let verifier = body.code_verifier.expose();
    let pkce_ok = verifier_is_valid(verifier)
        && tokens::constant_time_eq(&tokens::pkce_challenge(verifier), &code.code_challenge);
    if code.redirect_uri == body.redirect_uri && pkce_ok {
        Ok(())
    } else {
        Err(ApiError::TokenInvalid)
    }
}

/// `POST /devices/token` (no authentication, rate limited) → the device token. The code
/// is consumed first, so a failed attempt burns it.
pub async fn token(
    State(state): State<AppState>,
    Client(client): Client,
    ApiJson(body): ApiJson<DeviceTokenRequest>,
) -> Result<Json<DeviceTokenResponse>, ApiError> {
    limits::enforce(
        &state,
        &format!("device-token:{}", client.ip),
        limits::DEVICE_TOKEN_PER_IP,
    )
    .await?;
    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    let code = codes::consume(&tx, body.code.expose())
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    let user = users::find(&tx, code.user_id).await?;
    let outcome = check_exchange(&code, &body).and_then(|()| match &user {
        Some(user) => users::ensure_active(user),
        None => Err(ApiError::TokenInvalid),
    });
    if let Err(err) = outcome {
        // Keep the code burnt.
        tx.commit().await.map_err(anyhow::Error::from)?;
        return Err(err);
    }

    let ttl = Duration::from_secs(u64::try_from(code.token_ttl_secs).unwrap_or(0));
    let expires_at = crate::time::add(crate::time::now(), ttl)?;
    let (token, device) =
        devices::create(&tx, code.user_id, code.device_name, code.os, expires_at).await?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    tracing::info!(user_id = %device.user_id, device_id = %device.id, "device signed in");
    Ok(Json(DeviceTokenResponse {
        token: Sensitive::new(token.expose()),
        device_id: device.id.to_string(),
        expires_at: crate::time::rfc3339(device.expires_at),
    }))
}

/// `GET /devices` (session): the user's devices, newest first.
pub async fn list(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<Vec<DeviceInfo>>, ApiError> {
    let list = devices::list(&state.db, session.user.id).await?;
    Ok(Json(list.iter().map(device_info).collect()))
}

/// `PATCH /devices/{id}` (session) `{name}`.
pub async fn rename(
    State(state): State<AppState>,
    session: SessionUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateDeviceRequest>,
) -> Result<Json<DeviceInfo>, ApiError> {
    let id = parse_id(&id)?;
    let name = validate::text(&body.name, "name", validate::MAX_NAME_CHARS)?;
    if !devices::rename(&state.db, session.user.id, id, name).await? {
        return Err(ApiError::NotFound);
    }
    let device = devices::find(&state.db, session.user.id, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(device_info(&device)))
}

/// `DELETE /devices/{id}` (session) → 204: revokes the device token (soft).
pub async fn revoke(
    State(state): State<AppState>,
    session: SessionUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_id(&id)?;
    if devices::revoke(&state.db, session.user.id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
