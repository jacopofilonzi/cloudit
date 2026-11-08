//! Rate limits of the auth endpoints and of sharing (fixed windows, see `cache::rate_limit`: up to
//! twice the limit can pass across a window boundary, so the numbers are conservative).

use std::time::Duration;

use crate::cache::rate_limit::{RateDecision, RateLimit};
use crate::errors::ApiError;
use crate::state::AppState;

const MINUTE: u64 = 60;

/// `POST /auth/login`, per client IP.
pub const LOGIN_PER_IP: RateLimit = RateLimit::new(20, Duration::from_secs(5 * MINUTE));
/// `POST /auth/login`, per email (slows guessing one account from many IPs).
pub const LOGIN_PER_EMAIL: RateLimit = RateLimit::new(10, Duration::from_secs(15 * MINUTE));
/// `POST /auth/register`, per client IP.
pub const REGISTER_PER_IP: RateLimit = RateLimit::new(10, Duration::from_secs(60 * MINUTE));
/// `POST /setup`, per client IP.
pub const SETUP_PER_IP: RateLimit = RateLimit::new(10, Duration::from_secs(15 * MINUTE));
/// `POST /devices/token`, per client IP.
pub const DEVICE_TOKEN_PER_IP: RateLimit = RateLimit::new(30, Duration::from_secs(15 * MINUTE));
/// `POST /auth/password-reset`, per client IP.
pub const PASSWORD_RESET_PER_IP: RateLimit = RateLimit::new(10, Duration::from_secs(15 * MINUTE));
/// Current-password checks of a signed-in user (password change, email change).
pub const PASSWORD_CHECK_PER_USER: RateLimit = RateLimit::new(10, Duration::from_secs(15 * MINUTE));
/// Provider sign-in starts (each stores a flow), per client IP.
pub const OAUTH_START_PER_IP: RateLimit = RateLimit::new(30, Duration::from_secs(5 * MINUTE));
/// `POST /auth/invites/lookup`, per client IP (each attempt checks a token).
pub const INVITE_LOOKUP_PER_IP: RateLimit = RateLimit::new(30, Duration::from_secs(15 * MINUTE));
/// `POST /scopes/{id}/members`, per user: each attempt looks an email up.
pub const MEMBER_ADD_PER_USER: RateLimit = RateLimit::new(30, Duration::from_secs(15 * MINUTE));

/// Counts one hit on `key`; `rate_limited` (429 with `Retry-After`) when over `limit`.
pub async fn enforce(state: &AppState, key: &str, limit: RateLimit) -> Result<(), ApiError> {
    match state.rate_limiter.hit(key, limit).await {
        RateDecision::Allowed { .. } => Ok(()),
        RateDecision::Limited { retry_after } => {
            // Only the kind of limit: keys can hold an email or an IP.
            tracing::info!(
                limit = key.split(':').next().unwrap_or_default(),
                "rate limited"
            );
            Err(ApiError::RateLimited { retry_after })
        }
    }
}
