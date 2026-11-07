//! `/api/v1/me`: the signed-in user's profile, password and web sessions.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::protocol::{ChangePasswordRequest, SessionInfo, UpdateMeRequest, UserProfile};

use super::extract::{ApiJson, parse_id, profile, session_info};
use crate::accounts::users;
use crate::auth::extractors::{AnyUser, Client, SessionUser};
use crate::auth::{limits, password, sessions};
use crate::db::entities::users as user_entity;
use crate::errors::ApiError;
use crate::state::AppState;
use crate::validate;

/// Checks `current_password` against the account's password (rate limited per user).
/// Accounts without a password need none.
async fn check_current_password(
    state: &AppState,
    user: &user_entity::Model,
    current: Option<&str>,
) -> Result<(), ApiError> {
    let Some(stored) = user.password_hash.as_deref() else {
        return Ok(());
    };
    let current = current.ok_or_else(|| ApiError::validation("current_password", "required"))?;
    limits::enforce(
        state,
        &format!("password-check:{}", user.id),
        limits::PASSWORD_CHECK_PER_USER,
    )
    .await?;
    if current.len() > password::MAX_LEN
        || !password::verify_or_dummy(current, Some(stored)).await?
    {
        return Err(ApiError::InvalidCredentials);
    }
    Ok(())
}

/// `GET /me` (session or device).
pub async fn get(user: AnyUser) -> Json<UserProfile> {
    Json(profile(&user.user))
}

/// `PATCH /me` (session): display name and/or email. Changing the email needs
/// `current_password` when the account has one, and makes it unverified.
pub async fn update(
    State(state): State<AppState>,
    session: SessionUser,
    ApiJson(body): ApiJson<UpdateMeRequest>,
) -> Result<Json<UserProfile>, ApiError> {
    let user = &session.user;
    let display_name = body
        .display_name
        .as_deref()
        .map(validate::display_name)
        .transpose()?;
    let email = body
        .email
        .as_deref()
        .map(|e| validate::email(e, "email"))
        .transpose()?
        .filter(|e| *e != user.email);
    if email.is_some() {
        check_current_password(
            &state,
            user,
            body.current_password.as_ref().map(|p| p.expose()),
        )
        .await?;
    }
    if display_name.is_some() || email.is_some() {
        users::update_profile(&state.db, user.id, display_name, email).await?;
    }
    let updated = users::find(&state.db, user.id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(profile(&updated)))
}

/// `POST /me/password` (session) → 204. Signs out the account's other web sessions.
pub async fn change_password(
    State(state): State<AppState>,
    session: SessionUser,
    ApiJson(body): ApiJson<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    check_current_password(
        &state,
        &session.user,
        body.current_password.as_ref().map(|p| p.expose()),
    )
    .await?;
    password::validate(body.new_password.expose(), "new_password")?;
    let hash = password::hash(body.new_password.expose()).await?;
    users::set_password(&state.db, session.user.id, hash).await?;
    sessions::delete_all(&state.db, session.user.id, Some(session.session_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/sessions` (session): unexpired web sessions, `current` marked.
pub async fn list_sessions(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    let list = sessions::list(&state.db, session.user.id).await?;
    Ok(Json(
        list.iter()
            .map(|s| session_info(s, session.session_id))
            .collect(),
    ))
}

/// `DELETE /me/sessions/{id}` (session) → 204; deleting the current session also clears
/// the cookie.
pub async fn delete_session(
    State(state): State<AppState>,
    Client(client): Client,
    session: SessionUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_id(&id)?;
    if !sessions::delete(&state.db, session.user.id, id).await? {
        return Err(ApiError::NotFound);
    }
    if id == session.session_id {
        let cookie = sessions::clear_cookie(client.https)?;
        return Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
