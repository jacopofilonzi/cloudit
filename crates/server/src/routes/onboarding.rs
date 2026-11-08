//! `/api/v1/auth/onboarding*`: the first sign-in of an identity no account owns. Every
//! endpoint needs the `cloudit_onboarding` cookie of an unused, unexpired pending
//! signup (else `onboarding_expired`); the POSTs are CSRF-checked like all others.
//!
//! - `GET` → what the page shows.
//! - `POST /create`: a new account (registration mode and invites as for
//!   `/auth/register`; the email is the provider's unless another is typed, and only
//!   the provider's verified email, or an invite bound to the email, makes it
//!   verified; the password is optional). One transaction consumes the pending signup,
//!   creates the user and the identity, and consumes the invite.
//! - `POST /link`: attach the identity to an existing account proven with email and
//!   password (same rate limits and constant work as `/auth/login`).
//! - `POST /link/{provider}/start`: prove an existing account with another provider
//!   instead (an `onboarding_link` flow, see `oauth::complete`).
//!
//! Success signs in (session cookie) and clears the onboarding cookie. A new account
//! that needs an admin's approval (no invite, the provider's `new_accounts` is
//! `approval` and the identity is not in its admin groups) gets no session: 202.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::protocol::{
    ApprovalStatus, OnboardingCreateRequest, OnboardingInfo, OnboardingLinkRequest,
    OnboardingProvider, RegistrationMode,
};
use sea_orm::TransactionTrait as _;

use super::auth::pending_approval;
use super::extract::{ApiJson, profile};
use super::oauth::{active_provider, begin};
use crate::accounts::{invites, settings, users};
use crate::auth::client::ClientInfo;
use crate::auth::extractors::Client;
use crate::auth::{limits, password, sessions, tokens};
use crate::db::entities::{pending_signups, users as user_entity};
use crate::errors::ApiError;
use crate::idp::store::{self, Provider};
use crate::oauth::complete::{admin_by_groups, apply_groups};
use crate::oauth::flows::{NewFlow, Purpose};
use crate::oauth::{cookies, identities, pending};
use crate::state::AppState;
use crate::validate;

/// The pending signup of the request's onboarding cookie.
async fn current(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<pending_signups::Model, ApiError> {
    let token = cookies::read(headers, cookies::ONBOARDING).ok_or(ApiError::OnboardingExpired)?;
    pending::find_usable(&state.db, token.expose())
        .await?
        .ok_or(ApiError::OnboardingExpired)
}

/// Session cookie for `user`, onboarding cookie cleared, profile body.
async fn signed_in(
    state: &AppState,
    client: &ClientInfo,
    user: &user_entity::Model,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let (token, _) = sessions::create(&state.db, user.id, client.user_agent.as_deref()).await?;
    let mut response = (status, Json(profile(user))).into_response();
    let out = response.headers_mut();
    out.append(
        header::SET_COOKIE,
        sessions::set_cookie(&token, client.https)?,
    );
    out.append(
        header::SET_COOKIE,
        cookies::clear(cookies::ONBOARDING, client.https)?,
    );
    Ok(response)
}

async fn invite_usable(state: &AppState, hash: Option<&str>) -> anyhow::Result<bool> {
    Ok(match hash {
        Some(hash) => invites::find_usable_by_hash(&state.db, hash)
            .await?
            .is_some(),
        None => false,
    })
}

/// `GET /auth/onboarding`.
pub async fn info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OnboardingInfo>, ApiError> {
    let pending = current(&state, &headers).await?;
    let provider = store::find_any(&state.db, &pending.provider_id)
        .await?
        .ok_or(ApiError::OnboardingExpired)?;
    let mode = settings::registration_mode(&state.db).await?;
    let invite_carried = invite_usable(&state, pending.invite_token_hash.as_deref()).await?;
    let invite_required = mode == RegistrationMode::Invite && !invite_carried;
    let identity = pending::identity(&pending)?;
    let requires_approval = !invite_carried
        && users::approval_for_signup(
            provider.fields.new_accounts,
            admin_by_groups(&provider, &identity),
        ) == ApprovalStatus::Pending;
    Ok(Json(OnboardingInfo {
        provider: OnboardingProvider {
            id: provider.id.clone(),
            kind: provider.kind,
            display_name: provider.fields.display_name.clone(),
            icon: provider.fields.icon.clone(),
        },
        email_from_provider: pending.email.is_some(),
        email: pending.email,
        email_verified: pending.email_verified,
        display_name: pending.display_name,
        registration_mode: mode,
        invite_required,
        requires_approval,
    }))
}

/// `POST /auth/onboarding/create` → 201 + session cookie, or 202 `SignupPending`
/// (no session; the onboarding cookie is cleared) when the account needs approval.
pub async fn create(
    State(state): State<AppState>,
    Client(client): Client,
    headers: HeaderMap,
    ApiJson(body): ApiJson<OnboardingCreateRequest>,
) -> Result<Response, ApiError> {
    limits::enforce(
        &state,
        &format!("register:{}", client.ip),
        limits::REGISTER_PER_IP,
    )
    .await?;
    let pending = current(&state, &headers).await?;
    let provider = active_provider(&state, &pending.provider_id).await?;
    let mode = settings::registration_mode(&state.db).await?;
    let display_name = validate::display_name(&body.display_name)?;
    let email = match body.email.as_deref() {
        Some(raw) => validate::email(raw, "email")?,
        None => pending
            .email
            .clone()
            .ok_or_else(|| ApiError::validation("email", "required"))?,
    };
    let password_hash = match &body.password {
        Some(pw) => {
            password::validate(pw.expose(), "password")?;
            Some(password::hash(pw.expose()).await?)
        }
        None => None,
    };
    let invite_hash = body
        .invite_token
        .as_ref()
        .map(|t| tokens::hash_token(t.expose()))
        .or_else(|| pending.invite_token_hash.clone());
    let invite = match (invite_hash, mode) {
        (Some(hash), _) => {
            let invite = invites::find_usable_by_hash(&state.db, &hash)
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
    let identity = pending::identity(&pending)?;
    // Verified only if it is the provider's verified email, or an invite bound to it.
    let email_verified = (pending.email_verified
        && pending.email.as_deref() == Some(email.as_str()))
        || invite.as_ref().is_some_and(|i| i.email.is_some());
    // An invite is an admin's approval; an OIDC identity in the admin groups is
    // vouched for by the provider; otherwise the provider's `new_accounts` decides.
    let admin_from_groups = admin_by_groups(&provider, &identity);
    let (approval, source) = if invite.is_some() {
        (ApprovalStatus::Approved, "invite".to_owned())
    } else {
        (
            users::approval_for_signup(provider.fields.new_accounts, admin_from_groups),
            provider.id.clone(),
        )
    };

    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    if !pending::consume(&tx, pending.id).await? {
        return Err(ApiError::OnboardingExpired);
    }
    let user = users::insert(
        &tx,
        users::NewUser {
            email,
            email_verified,
            display_name,
            password_hash,
            admin: false,
            approval,
            signup_source: Some(source),
        },
    )
    .await?;
    identities::insert(&tx, user.id, &provider.id, &identity, true).await?;
    if let Some(invite) = invite
        && !invites::consume(&tx, invite.id, user.id).await?
    {
        return Err(ApiError::InviteInvalid);
    }
    if admin_from_groups {
        users::grant_admin_from_groups(&tx, user.id).await?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    if approval == ApprovalStatus::Pending {
        tracing::info!(user_id = %user.id, provider = %provider.id, "account created through a provider, waiting for approval");
        let mut response = pending_approval();
        response.headers_mut().append(
            header::SET_COOKIE,
            cookies::clear(cookies::ONBOARDING, client.https)?,
        );
        return Ok(response);
    }
    let user = users::find(&state.db, user.id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    tracing::info!(user_id = %user.id, provider = %provider.id, "account created through a provider");
    signed_in(&state, &client, &user, StatusCode::CREATED).await
}

async fn attach(
    state: &AppState,
    pending: &pending_signups::Model,
    provider: &Provider,
    user: &user_entity::Model,
) -> Result<(), ApiError> {
    let identity = pending::identity(pending)?;
    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    if !pending::consume(&tx, pending.id).await? {
        return Err(ApiError::OnboardingExpired);
    }
    identities::insert(&tx, user.id, &provider.id, &identity, true).await?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    apply_groups(state, provider, user.id, &identity).await?;
    tracing::info!(user_id = %user.id, provider = %provider.id, "pending identity linked with a password");
    Ok(())
}

/// `POST /auth/onboarding/link` → 200 + session cookie. Wrong email and wrong password
/// give the same `invalid_credentials`, after the same amount of work.
pub async fn link_with_password(
    State(state): State<AppState>,
    Client(client): Client,
    headers: HeaderMap,
    ApiJson(body): ApiJson<OnboardingLinkRequest>,
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
    let pending = current(&state, &headers).await?;
    let provider = active_provider(&state, &pending.provider_id).await?;
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
    users::ensure_active(&user)?;
    attach(&state, &pending, &provider, &user).await?;
    let user = users::find(&state.db, user.id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    signed_in(&state, &client, &user, StatusCode::OK).await
}

/// `POST /auth/onboarding/link/{provider}/start` → `{redirect_url}`: prove an existing
/// account by signing in with `provider`.
pub async fn link_with_provider(
    State(state): State<AppState>,
    Client(client): Client,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let pending = current(&state, &headers).await?;
    let provider = active_provider(&state, &provider_id).await?;
    begin(
        &state,
        &client,
        &headers,
        &provider,
        NewFlow {
            provider_id: &provider.id,
            purpose: Purpose::OnboardingLink,
            user_id: None,
            pending_signup_id: Some(pending.id),
            return_to: "/".to_owned(),
            invite_token_hash: None,
        },
    )
    .await
}
