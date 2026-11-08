//! `/api/v1/auth/*`: login page config, registration, login, logout, password reset.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::protocol::{
    ApprovalStatus, AuthConfig, LoginRequest, NewAccounts, PasswordResetRequest, RegisterRequest,
    RegistrationMode, SignupPending,
};
use sea_orm::TransactionTrait as _;

use super::extract::{ApiJson, profile};
use crate::accounts::{invites, password_resets, settings, users};
use crate::auth::client::ClientInfo;
use crate::auth::extractors::{Client, SessionUser};
use crate::auth::middleware::setup_pending;
use crate::auth::{limits, password, sessions};
use crate::db::entities::users as user_entity;
use crate::errors::ApiError;
use crate::state::AppState;
use crate::validate;

/// Creates a session for `user` and answers its profile with the session cookie.
pub async fn signed_in(
    state: &AppState,
    client: &ClientInfo,
    user: &user_entity::Model,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let (token, _) = sessions::create(&state.db, user.id, client.user_agent.as_deref()).await?;
    let cookie = sessions::set_cookie(&token, client.https)?;
    Ok((status, [(header::SET_COOKIE, cookie)], Json(profile(user))).into_response())
}

/// 202 `{status: "pending"}`: the new account waits for an admin; no session.
pub fn pending_approval() -> Response {
    (StatusCode::ACCEPTED, Json(SignupPending::default())).into_response()
}

/// `GET /auth/config` (no authentication; works during setup).
pub async fn config(State(state): State<AppState>) -> Result<Json<AuthConfig>, ApiError> {
    Ok(Json(AuthConfig {
        setup_required: setup_pending(&state).await?,
        registration_mode: settings::registration_mode(&state.db).await?,
        password_requires_approval: settings::password_new_accounts(&state.db).await?
            == NewAccounts::Approval,
        providers: crate::idp::store::list(&state.db, true)
            .await?
            .iter()
            .map(crate::idp::store::Provider::public)
            .collect(),
    }))
}

/// `POST /auth/register` → 201 + session cookie, or 202 [`SignupPending`] (no
/// session) when the account needs an admin's approval: without an invite and with
/// the `password_new_accounts` setting at `approval`.
pub async fn register(
    State(state): State<AppState>,
    Client(client): Client,
    ApiJson(body): ApiJson<RegisterRequest>,
) -> Result<Response, ApiError> {
    limits::enforce(
        &state,
        &format!("register:{}", client.ip),
        limits::REGISTER_PER_IP,
    )
    .await?;
    let mode = settings::registration_mode(&state.db).await?;
    let email = validate::email(&body.email, "email")?;
    let display_name = validate::display_name(&body.display_name)?;
    password::validate(body.password.expose(), "password")?;

    let invite = match (&body.invite_token, mode) {
        (Some(token), _) => {
            let invite = invites::find_usable(&state.db, token.expose())
                .await?
                .ok_or(ApiError::InviteInvalid)?;
            if invite.email.as_ref().is_some_and(|bound| *bound != email) {
                return Err(ApiError::InviteInvalid);
            }
            Some(invite)
        }
        (None, RegistrationMode::Invite) => return Err(ApiError::InviteInvalid),
        (None, _) => None,
    };
    // Only an invite bound to this very email proves the address.
    let email_verified = invite.as_ref().is_some_and(|i| i.email.is_some());
    // An invite is an admin's approval; otherwise the setting decides.
    let (approval, source) = if invite.is_some() {
        (ApprovalStatus::Approved, "invite")
    } else {
        let policy = settings::password_new_accounts(&state.db).await?;
        (users::approval_for_signup(policy, false), "password")
    };

    let password_hash = password::hash(body.password.expose()).await?;
    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    let user = users::insert(
        &tx,
        users::NewUser {
            email,
            email_verified,
            display_name,
            password_hash: Some(password_hash),
            admin: false,
            approval,
            signup_source: Some(source.to_owned()),
        },
    )
    .await?;
    if let Some(invite) = invite
        && !invites::consume(&tx, invite.id, user.id).await?
    {
        // Used or revoked meanwhile: the transaction (and the user) is rolled back.
        return Err(ApiError::InviteInvalid);
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    if approval == ApprovalStatus::Pending {
        tracing::info!(user_id = %user.id, "account registered, waiting for approval");
        return Ok(pending_approval());
    }
    tracing::info!(user_id = %user.id, "account registered");
    signed_in(&state, &client, &user, StatusCode::CREATED).await
}

/// `POST /auth/login` → 200 + session cookie. Wrong email and wrong password give the
/// same `invalid_credentials`, after the same amount of work.
pub async fn login(
    State(state): State<AppState>,
    Client(client): Client,
    ApiJson(body): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    limits::enforce(
        &state,
        &format!("login:{}", client.ip),
        limits::LOGIN_PER_IP,
    )
    .await?;
    let email = validate::email(&body.email, "email").ok();
    if let Some(email) = &email {
        limits::enforce(
            &state,
            &format!("login-email:{email}"),
            limits::LOGIN_PER_EMAIL,
        )
        .await?;
    }
    if body.password.expose().len() > password::MAX_LEN {
        return Err(ApiError::InvalidCredentials);
    }
    let user = match &email {
        Some(email) => users::find_by_email(&state.db, email).await?,
        None => None,
    };
    let stored = user.as_ref().and_then(|u| u.password_hash.as_deref());
    let valid = password::verify_or_dummy(body.password.expose(), stored).await?;
    let Some(user) = user.filter(|_| valid) else {
        return Err(ApiError::InvalidCredentials);
    };
    // Only after the password check: no oracle beyond what the password proves.
    users::ensure_active(&user)?;
    signed_in(&state, &client, &user, StatusCode::OK).await
}

/// `POST /auth/logout` → 204; deletes the current session (if any) and clears the
/// cookie.
pub async fn logout(
    State(state): State<AppState>,
    Client(client): Client,
    session: Result<SessionUser, ApiError>,
) -> Result<Response, ApiError> {
    if let Ok(session) = session {
        sessions::delete(&state.db, session.user.id, session.session_id).await?;
    }
    let cookie = sessions::clear_cookie(client.https)?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

/// `POST /auth/password-reset` → 204. Sets the password and signs out every web session
/// of the account (devices stay); does not sign in: the user logs in with the new
/// password.
pub async fn password_reset(
    State(state): State<AppState>,
    Client(client): Client,
    ApiJson(body): ApiJson<PasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    limits::enforce(
        &state,
        &format!("reset:{}", client.ip),
        limits::PASSWORD_RESET_PER_IP,
    )
    .await?;
    password::validate(body.new_password.expose(), "new_password")?;
    let password_hash = password::hash(body.new_password.expose()).await?;

    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    let user_id = password_resets::consume(&tx, body.token.expose())
        .await?
        .ok_or(ApiError::TokenInvalid)?;
    users::set_password(&tx, user_id, password_hash).await?;
    sessions::delete_all(&tx, user_id, None).await?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    tracing::info!(%user_id, "password reset");
    Ok(StatusCode::NO_CONTENT)
}
