//! `cloudit-server`: the stateless HTTP server of cloudit.
//!
//! The binary (`main.rs`) only wires things together; everything lives in this
//! library so integration tests can build the router without a socket.
//!
//! - [`config`]: configuration read once from the environment.
//! - [`state`]: [`state::AppState`], the shared handles built at startup.
//! - [`db`]: SeaORM connection (SQLite or PostgreSQL), migrations, leases.
//! - [`storage`]: [`storage::BlobStore`] over `object_store` (filesystem or S3).
//! - [`cache`]: optional Redis and the rate limiter.
//! - [`crypto`]: the server key and encryption of stored secrets.
//! - [`accounts`]: users, first-run setup, settings, invites, password resets.
//! - [`auth`]: passwords, sessions, device tokens, CSRF, extractors, rate limits.
//! - [`devices`]: desktop devices and their loopback + PKCE login.
//! - [`idp`]: identity providers (presets, configuration, OIDC discovery, HTTP client,
//!   code exchange and ID token verification).
//! - [`oauth`]: provider sign-in state (flows, identities, pending signups) and outcomes.
//! - [`scopes`]: scopes, members, folders, the trash and the access checks.
//! - [`cli`]: `cloudit-server` subcommands (`setup-token`, `admin reset-password`).
//! - [`validate`], [`time`]: input validation and timestamp helpers.
//! - [`errors`]: [`errors::ApiError`] and its JSON shape with stable error codes.
//! - [`routes`]: the axum router (health probes, `/api/v1`, SPA static files).
//! - [`logging`]: tracing subscriber setup (`pretty` or `json`).
//! - [`shutdown`], [`platform`]: graceful-shutdown signal and private files (the only
//!   OS-specific code).
//! - [`healthcheck`]: `cloudit-server healthcheck` probe for the Docker `HEALTHCHECK`.

pub mod accounts;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod db;
pub mod devices;
pub mod errors;
pub mod healthcheck;
pub mod idp;
pub mod logging;
pub mod oauth;
pub mod platform;
pub mod routes;
pub mod scopes;
pub mod shutdown;
pub mod state;
pub mod storage;
pub mod time;
pub mod validate;
