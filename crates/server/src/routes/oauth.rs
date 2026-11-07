//! `/api/v1/auth/oauth/{provider}/*`: provider sign-in.
//!
//! - `POST …/start` (CSRF-checked like every POST: a cross-site GET could otherwise
//!   start a `link` flow inside the victim's session) → `{redirect_url}` and the
//!   `cloudit_oauth` browser-binding cookie.
//! - `GET …/callback?code&state` (browser navigation): answers with redirects, never
//!   JSON: the return path (signed in or linked), `/onboarding` (unknown identity), or
//!   `/login?error=<code>` on any failure.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::protocol::{OAuthPurpose, OAuthStartRequest, OAuthStartResponse};

use super::extract::ApiJson;
use crate::auth::client::ClientInfo;
use crate::auth::extractors::{Client, SessionUser};
use crate::auth::{limits, sessions, tokens};
use crate::errors::ApiError;
use crate::idp::signin::AuthRequest;
use crate::idp::store::{self, Provider, callback_url};
use crate::oauth::complete::{self, Outcome};
use crate::oauth::flows::{self, NewFlow, Purpose};
use crate::oauth::{cookies, pending};
use crate::state::AppState;

/// A provider usable for sign-in (`provider_not_found` / `provider_disabled`).
pub async fn active_provider(state: &AppState, id: &str) -> Result<Provider, ApiError> {
    let provider = store::find(&state.db, id).await?;
    if !provider.fields.enabled {
        return Err(ApiError::ProviderDisabled);
    }
    Ok(provider)
}

/// The callback URL of `provider`; `public_url_required` without a public URL.
async fn redirect_uri(state: &AppState, provider: &Provider) -> Result<String, ApiError> {
    let base = state
        .public_url()
        .await?
        .ok_or(ApiError::PublicUrlRequired)?;
    Ok(callback_url(&base, &provider.id))
}

/// Starts a flow: stores it, sets the binding cookie, answers the authorization URL.
pub async fn begin(
    state: &AppState,
    client: &ClientInfo,
    headers: &HeaderMap,
    provider: &Provider,
    new: NewFlow<'_>,
) -> Result<Response, ApiError> {
    limits::enforce(
        state,
        &format!("oauth-start:{}", client.ip),
        limits::OAUTH_START_PER_IP,
    )
    .await?;
    let redirect_uri = redirect_uri(state, provider).await?;
    let binding = match cookies::read(headers, cookies::BINDING) {
        Some(existing) => existing,
        None => tokens::new_token()?,
    };
    let started = flows::create(&state.db, new, &binding).await?;
    let url = state
        .idp
        .authorize_url(
            provider,
            &AuthRequest {
                redirect_uri: &redirect_uri,
                state: started.state.expose(),
                nonce: started.nonce.expose(),
                code_challenge: &tokens::pkce_challenge(started.verifier.expose()),
            },
        )
        .await?;
    let cookie = cookies::set(cookies::BINDING, &binding, flows::TTL, client.https)?;
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(OAuthStartResponse { redirect_url: url }),
    )
        .into_response())
}

/// `POST /auth/oauth/{provider}/start` → `{redirect_url}`. `link` needs a web session.
pub async fn start(
    State(state): State<AppState>,
    Client(client): Client,
    session: Result<SessionUser, ApiError>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<OAuthStartRequest>,
) -> Result<Response, ApiError> {
    let return_to = crate::oauth::return_to(body.return_to.as_deref())?;
    let (purpose, user_id) = match body.purpose {
        OAuthPurpose::Login => (Purpose::Login, None),
        OAuthPurpose::Link => (Purpose::Link, Some(session?.user.id)),
    };
    let invite_token_hash = match (&body.invite_token, purpose) {
        (Some(token), Purpose::Login) => Some(tokens::hash_token(token.expose())),
        _ => None,
    };
    let provider = active_provider(&state, &provider_id).await?;
    begin(
        &state,
        &client,
        &headers,
        &provider,
        NewFlow {
            provider_id: &provider.id,
            purpose,
            user_id,
            pending_signup_id: None,
            return_to,
            invite_token_hash,
        },
    )
    .await
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    match HeaderValue::from_str(location) {
        Ok(value) => {
            response.headers_mut().insert(header::LOCATION, value);
        }
        Err(_) => {
            response
                .headers_mut()
                .insert(header::LOCATION, HeaderValue::from_static("/"));
        }
    }
    response
}

fn error_redirect(err: &ApiError) -> Response {
    let code = serde_json::to_value(err.code())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "internal".to_owned());
    if let ApiError::Internal(cause) = err {
        tracing::error!(error = ?cause, "provider callback failed");
    } else {
        tracing::info!(code, "provider callback refused");
    }
    redirect(&format!("/login?error={code}"))
}

/// `GET /auth/oauth/{provider}/callback?code&state` (see the module docs).
pub async fn callback(
    State(state): State<AppState>,
    Client(client): Client,
    session: Result<SessionUser, ApiError>,
    Path(provider_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    match handle_callback(
        &state,
        &client,
        session.ok(),
        &provider_id,
        &params,
        &headers,
    )
    .await
    {
        Ok(response) => response,
        Err(err) => error_redirect(&err),
    }
}

async fn handle_callback(
    state: &AppState,
    client: &ClientInfo,
    session: Option<SessionUser>,
    provider_id: &str,
    params: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let flow_state = params.get("state").ok_or(ApiError::OauthStateInvalid)?;
    let binding = cookies::read(headers, cookies::BINDING).ok_or(ApiError::OauthStateInvalid)?;
    let flow = flows::consume(&state.db, provider_id, flow_state, binding.expose())
        .await?
        .ok_or(ApiError::OauthStateInvalid)?;
    if let Some(error) = params.get("error") {
        tracing::info!(provider = provider_id, error = %error, "provider refused the sign-in");
        return Err(if error == "access_denied" {
            ApiError::OauthCancelled
        } else {
            ApiError::ProviderError
        });
    }
    let code = params.get("code").ok_or(ApiError::ProviderError)?;
    let provider = active_provider(state, provider_id).await?;
    let redirect_uri = redirect_uri(state, &provider).await?;
    let secret = provider.client_secret(&state.secrets)?;
    let identity = state
        .idp
        .sign_in(
            &provider,
            &secret,
            code,
            &flow.pkce_verifier.0,
            &flow.nonce.0,
            &redirect_uri,
        )
        .await?;
    let session_user = session.map(|s| s.user.id);
    let outcome = complete::finish(state, &provider, &flow, &identity, session_user).await?;
    match outcome {
        Outcome::Linked => Ok(redirect(&flow.return_to)),
        Outcome::SignedIn {
            user_id,
            clear_onboarding,
        } => {
            let (token, _) =
                sessions::create(&state.db, user_id, client.user_agent.as_deref()).await?;
            let mut response = redirect(&flow.return_to);
            let out = response.headers_mut();
            out.append(
                header::SET_COOKIE,
                sessions::set_cookie(&token, client.https)?,
            );
            if clear_onboarding {
                out.append(
                    header::SET_COOKIE,
                    cookies::clear(cookies::ONBOARDING, client.https)?,
                );
            }
            tracing::info!(%user_id, provider = %provider.id, "signed in with a provider");
            Ok(response)
        }
        Outcome::Onboarding { token } => {
            let mut response = redirect("/onboarding");
            response.headers_mut().append(
                header::SET_COOKIE,
                cookies::set(cookies::ONBOARDING, &token, pending::TTL, client.https)?,
            );
            Ok(response)
        }
    }
}
