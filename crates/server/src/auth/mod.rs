//! Authentication: password accounts, web sessions, device tokens, CSRF, rate limits.
//!
//! - [`tokens`]: random tokens, SHA-256 storage form, PKCE, constant-time compare.
//! - [`password`]: argon2id hashing and verification (no timing oracle for unknown
//!   emails).
//! - [`sessions`]: the `cloudit_session` cookie and sliding-expiry sessions.
//! - [`client`]: client IP, https and own origin, honouring proxy headers only with
//!   `TRUST_PROXY_HEADERS`.
//! - [`csrf`]: `Origin` / `Sec-Fetch-Site` check for cookie requests.
//! - [`middleware`]: the `/api/v1` layer (setup gate, CSRF, credential resolution).
//! - [`extractors`]: `SessionUser`, `AnyUser`, `AdminUser`, `Client`: each route states
//!   what it accepts.
//! - [`limits`]: rate limits of the auth endpoints.
//!
//! Tokens of every kind are stored only as SHA-256 hashes; plaintext tokens live in
//! [`crate::config::Secret`] (or `Sensitive` in API bodies) and are never logged.

pub mod client;
pub mod csrf;
pub mod extractors;
pub mod limits;
pub mod middleware;
pub mod password;
pub mod sessions;
pub mod tokens;
