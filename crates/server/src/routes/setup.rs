//! `/api/v1/setup`: first-run creation of the first admin (see `accounts::setup`).

use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use cloudit_core::protocol::{SetupRequest, SetupStatus};
use sea_orm::TransactionTrait as _;

use super::auth::signed_in;
use super::extract::ApiJson;
use crate::accounts::{settings, setup, users};
use crate::auth::extractors::Client;
use crate::auth::middleware::setup_pending;
use crate::auth::{limits, password};
use crate::errors::ApiError;
use crate::state::AppState;
use crate::validate;

/// `GET /setup` → `{required}`.
pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, ApiError> {
    Ok(Json(SetupStatus {
        required: setup_pending(&state).await?,
    }))
}

/// `POST /setup` → 201 + session cookie. Consumes the setup token, creates the first
/// admin (email verified) and stores the registration mode, all in one transaction.
pub async fn complete(
    State(state): State<AppState>,
    Client(client): Client,
    ApiJson(body): ApiJson<SetupRequest>,
) -> Result<Response, ApiError> {
    limits::enforce(
        &state,
        &format!("setup:{}", client.ip),
        limits::SETUP_PER_IP,
    )
    .await?;
    // Fail fast (and in constant time) before hashing the password.
    setup::check_token(&state.db, body.token.expose()).await?;
    let email = validate::email(&body.email, "email")?;
    let display_name = validate::display_name(&body.display_name)?;
    password::validate(body.password.expose(), "password")?;
    let public_url = match body.public_url.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(url) => Some(validate::public_url(url, "public_url")?),
    };
    let password_hash = password::hash(body.password.expose()).await?;

    let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    setup::complete(&tx, body.token.expose()).await?;
    let user = users::insert(
        &tx,
        users::NewUser {
            email,
            email_verified: true,
            display_name,
            password_hash: Some(password_hash),
            admin: true,
        },
    )
    .await?;
    settings::set_registration_mode(
        &tx,
        body.registration_mode.unwrap_or_default(),
        Some(user.id),
    )
    .await?;
    if let Some(url) = &public_url {
        settings::set_public_url(&tx, Some(url), Some(user.id)).await?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    state.setup_done.store(true, Ordering::Relaxed);
    tracing::info!(user_id = %user.id, "first-run setup completed");
    signed_in(&state, &client, &user, StatusCode::CREATED).await
}
