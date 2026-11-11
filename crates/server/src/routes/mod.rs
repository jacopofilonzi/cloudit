//! HTTP router.
//!
//! Layout:
//! - `GET /health` (liveness) and `GET /ready` (readiness) at the root, for probes
//!   and the Docker `HEALTHCHECK`;
//! - the versioned API under `/api/v1`; anything else under `/api` answers a JSON
//!   `not_found` error instead of falling through to the SPA;
//! - everything else serves the built web UI from `WEB_DIR`, with `index.html` as the
//!   fallback so client-side routes work on reload. If `WEB_DIR` is missing (e.g. API
//!   only, or the UI isn't built yet) a warning is logged and those paths 404.
//!
//! Every `/api/v1` route runs behind `auth::middleware::layer` (setup gate, CSRF,
//! credentials); each handler states what it accepts through its extractors
//! (`SessionUser`, `AnyUser`, `AdminUser` or none). The route table with the auth
//! requirement of each endpoint is in STRUCTURE.md ("API").

mod admin;
mod auth;
mod chunks;
mod devices;
mod events;
mod extract;
mod folders;
mod health;
mod jobs;
mod me;
mod oauth;
mod onboarding;
mod providers;
mod scopes;
mod setup;
mod snapshots;
mod trash;
mod usage;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::errors::ApiError;
pub use crate::state::AppState;

/// Body limit of a commit: the stored manifest is a zstd frame of at most
/// `DEFAULT_MAX_MANIFEST_BYTES` of JSON, which compresses (an incompressible body would
/// fail to decode anyway).
const MAX_MANIFEST_BODY: usize = cloudit_core::manifest::DEFAULT_MAX_MANIFEST_BYTES;

/// Builds the complete application router.
pub fn router(state: AppState) -> Router {
    let api_v1 = Router::new()
        .route("/health", get(health::health))
        .route("/setup", get(setup::status).post(setup::complete))
        .route("/auth/config", get(auth::config))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/password-reset", post(auth::password_reset))
        .route("/auth/invites/lookup", post(auth::lookup_invite))
        .route("/auth/oauth/{provider}/start", post(oauth::start))
        .route("/auth/oauth/{provider}/callback", get(oauth::callback))
        .route("/me", get(me::get).patch(me::update))
        .route("/me/password", post(me::change_password))
        .route("/me/sessions", get(me::list_sessions))
        .route("/me/sessions/{id}", delete(me::delete_session))
        .route("/me/identities", get(me::list_identities))
        .route("/me/identities/{id}", delete(me::unlink_identity))
        .route("/me/usage", get(usage::get))
        .route("/auth/onboarding", get(onboarding::info))
        .route("/auth/onboarding/create", post(onboarding::create))
        .route(
            "/auth/onboarding/link",
            post(onboarding::link_with_password),
        )
        .route(
            "/auth/onboarding/link/{provider}/start",
            post(onboarding::link_with_provider),
        )
        .route("/devices", get(devices::list))
        .route("/devices/authorize", post(devices::authorize))
        .route("/devices/token", post(devices::token))
        .route(
            "/devices/{id}",
            patch(devices::rename).delete(devices::revoke),
        )
        .route("/admin/users", get(admin::list_users))
        .route("/admin/users/{id}", patch(admin::update_user))
        .route("/admin/users/{id}/approve", post(admin::approve_user))
        .route("/admin/users/{id}/reject", post(admin::reject_user))
        .route("/admin/summary", get(admin::summary))
        .route("/admin/jobs", get(jobs::list))
        .route("/admin/jobs/{name}/run", post(jobs::run))
        .route(
            "/admin/users/{id}/password-reset",
            post(admin::create_password_reset),
        )
        .route(
            "/admin/invites",
            get(admin::list_invites).post(admin::create_invite),
        )
        .route("/admin/invites/{id}", delete(admin::revoke_invite))
        .route(
            "/admin/settings",
            get(admin::get_settings).patch(admin::update_settings),
        )
        .route(
            "/admin/providers",
            get(providers::list).post(providers::create),
        )
        .route("/admin/providers/presets", get(providers::presets))
        .route(
            "/admin/providers/{id}",
            patch(providers::update).delete(providers::delete),
        )
        .route("/admin/providers/{id}/test", post(providers::test))
        .route("/scopes", get(scopes::list).post(scopes::create))
        .route(
            "/scopes/{id}",
            get(scopes::get)
                .patch(scopes::update)
                .delete(scopes::delete),
        )
        .route("/scopes/{id}/restore", post(scopes::restore))
        .route(
            "/scopes/{id}/members",
            get(scopes::list_members).post(scopes::add_member),
        )
        .route(
            "/scopes/{id}/members/{user_id}",
            patch(scopes::update_member).delete(scopes::remove_member),
        )
        .route(
            "/scopes/{id}/folders",
            get(folders::list).post(folders::create),
        )
        .route(
            "/folders/{id}",
            get(folders::get)
                .patch(folders::update)
                .delete(folders::delete),
        )
        .route("/folders/{id}/restore", post(folders::restore))
        .route("/folders/{id}/chunks/check", post(chunks::check))
        .route("/folders/{id}/chunks/download", post(chunks::download_urls))
        .route(
            "/folders/{id}/chunks/pack",
            post(chunks::upload_pack)
                .layer(DefaultBodyLimit::max(cloudit_core::pack::MAX_PACK_BYTES)),
        )
        .route(
            "/folders/{id}/chunks/pack/download",
            post(chunks::download_pack),
        )
        .route(
            "/folders/{id}/chunks/{hash}",
            get(chunks::download)
                .put(chunks::upload)
                .layer(DefaultBodyLimit::max(
                    cloudit_core::protocol::MAX_STORED_CHUNK_BYTES,
                )),
        )
        .route(
            "/folders/{id}/snapshots",
            get(snapshots::list)
                .post(snapshots::commit)
                .layer(DefaultBodyLimit::max(MAX_MANIFEST_BODY)),
        )
        .route(
            "/snapshots/{id}",
            get(snapshots::get).patch(snapshots::update),
        )
        .route("/snapshots/{id}/manifest", get(snapshots::manifest))
        .route("/snapshots/{id}/archive", get(snapshots::archive))
        .route("/snapshots/{id}/restore", post(snapshots::restore))
        .route("/snapshots/{id}/resolve", post(snapshots::resolve))
        .route("/trash", get(trash::list))
        .route("/trash/scopes/{id}", delete(trash::purge_scope))
        .route("/trash/folders/{id}", delete(trash::purge_folder))
        .route("/trash/empty", post(trash::empty))
        .route("/events", get(events::stream))
        .route("/sync/heads", get(events::heads))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::layer,
        ));

    let api = Router::new()
        .nest("/v1", api_v1)
        .fallback(|| async { ApiError::NotFound });

    let app = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .nest("/api", api);

    let web_dir = &state.config.web_dir;
    let app = if web_dir.is_dir() {
        let index = web_dir.join("index.html");
        app.fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
    } else {
        tracing::warn!(
            web_dir = %web_dir.display(),
            "WEB_DIR does not exist: the web UI will not be served"
        );
        app.fallback(|| async { ApiError::NotFound })
    };

    app.layer(TraceLayer::new_for_http()).with_state(state)
}

#[cfg(test)]
mod tests;
