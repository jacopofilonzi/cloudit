//! The `/api/v1` middleware: runs before every API handler.
//!
//! 1. Computes [`ClientInfo`] (IP, https, own origin).
//! 2. **Setup gate**: while first-run setup is pending, everything except
//!    [`SETUP_OPEN_PATHS`] answers `setup_required` (409). Once setup is seen completed
//!    the answer is cached in memory (it can never become pending again: the
//!    `setup_state` row is never deleted), so completed servers pay no query.
//! 3. **CSRF** ([`super::csrf`]) for unsafe, non-bearer requests.
//! 4. **Authentication**: `Authorization: Bearer cld_…` → device; else the session
//!    cookie → session (bearer wins: an invalid bearer token is not rescued by a
//!    cookie). The result is an [`Auth`] extension that the extractors
//!    ([`super::extractors`]) turn into 401/403 as each route requires. Disabled,
//!    pending and rejected users still resolve, so the extractors can answer
//!    `account_disabled`, `account_pending` or `account_rejected`.
//! 5. After the handler: if the session was extended, refreshes the cookie (unless
//!    the handler set or cleared it itself).

use std::sync::atomic::Ordering;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};

use super::client::ClientInfo;
use super::{csrf, sessions};
use crate::accounts::setup;
use crate::db::entities::{devices as device_entity, sessions as session_entity, users};
use crate::devices;
use crate::errors::ApiError;
use crate::state::AppState;

/// Paths (relative to `/api/v1`) that work while setup is pending.
pub const SETUP_OPEN_PATHS: &[&str] = &["/health", "/auth/config", "/setup"];
/// Unsafe paths exempt from the CSRF check: called by native clients without cookies.
const CSRF_EXEMPT_PATHS: &[&str] = &["/devices/token"];

/// Who is making the request (an extension set by [`layer`]).
#[derive(Debug, Clone)]
pub enum Auth {
    Anonymous,
    Session {
        user: users::Model,
        session: session_entity::Model,
    },
    Device {
        user: users::Model,
        device: device_entity::Model,
    },
}

/// `true` while first-run setup is pending (see the module docs).
pub async fn setup_pending(state: &AppState) -> anyhow::Result<bool> {
    if state.setup_done.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let pending = setup::status(&state.db).await?.is_pending();
    if !pending {
        state.setup_done.store(true, Ordering::Relaxed);
    }
    Ok(pending)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

/// Steps 1–4; returns the cookie to refresh, if any.
async fn prepare(state: &AppState, request: &mut Request) -> Result<Option<String>, ApiError> {
    let client = ClientInfo::from_request(&state.config, request);
    let path = request.uri().path().to_owned();

    if !SETUP_OPEN_PATHS.contains(&path.as_str()) && setup_pending(state).await? {
        return Err(ApiError::SetupRequired);
    }

    let headers = request.headers();
    let bearer = bearer_token(headers).map(str::to_owned);
    if csrf::is_unsafe(request.method())
        && bearer.is_none()
        && !CSRF_EXEMPT_PATHS.contains(&path.as_str())
    {
        csrf::check(headers, client.origin.as_deref())?;
    }

    let now = crate::time::now();
    let mut refresh = None;
    let auth = if let Some(token) = bearer {
        match devices::authenticate(&state.db, &token).await? {
            Some((device, user)) => {
                devices::touch(&state.db, &device, now).await?;
                Auth::Device { user, device }
            }
            None => Auth::Anonymous,
        }
    } else if let Some(token) = sessions::read_cookie(headers) {
        match sessions::authenticate(&state.db, token.expose()).await? {
            Some((session, user)) => {
                if user.is_active() && sessions::slide_if_due(&state.db, &session, now).await? {
                    refresh = Some(token.expose().to_owned());
                }
                Auth::Session { user, session }
            }
            None => Auth::Anonymous,
        }
    } else {
        Auth::Anonymous
    };

    request.extensions_mut().insert(auth);
    request.extensions_mut().insert(client);
    Ok(refresh)
}

/// The middleware function (see the module docs).
pub async fn layer(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let refresh = match prepare(&state, &mut request).await {
        Ok(refresh) => refresh,
        Err(err) => return err.into_response(),
    };
    let https = request
        .extensions()
        .get::<ClientInfo>()
        .is_some_and(|c| c.https);
    let mut response = next.run(request).await;
    if let Some(token) = refresh
        && !sessions::sets_cookie(response.headers())
    {
        match sessions::set_cookie(&crate::config::Secret::new(token), https) {
            Ok(cookie) => {
                response.headers_mut().append(header::SET_COOKIE, cookie);
            }
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "refreshing the session cookie")
            }
        }
    }
    response
}
