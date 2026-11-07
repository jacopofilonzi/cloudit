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
//! - [`errors`]: [`errors::ApiError`] and its JSON shape with stable error codes.
//! - [`routes`]: the axum router (health probes, `/api/v1`, SPA static files).
//! - [`logging`]: tracing subscriber setup (`pretty` or `json`).
//! - [`shutdown`]: graceful-shutdown signal (the only OS-specific code).
//! - [`healthcheck`]: `cloudit-server healthcheck` probe for the Docker `HEALTHCHECK`.

pub mod cache;
pub mod config;
pub mod db;
pub mod errors;
pub mod healthcheck;
pub mod logging;
pub mod routes;
pub mod shutdown;
pub mod state;
pub mod storage;
