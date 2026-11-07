//! Identity providers (OIDC and OAuth2) configured by admins in the database.
//!
//! - [`presets`]: the static table of kinds (Google, Microsoft, GitLab, self-hosted
//!   OIDC, GitHub, Discord) with their protocol, issuer rules and defaults.
//! - [`fields`]: the admin-editable settings and their validation.
//! - [`store`]: the `auth_providers` table (secrets encrypted, soft delete).
//! - [`http`]: the shared outgoing HTTP client (timeouts, no redirects, bounded bodies).
//! - [`oidc`]: discovery and JWKS, cached in memory per issuer.
//! - [`signin`]: authorization URL, code exchange, ID token verification.
//! - [`social`]: GitHub and Discord user APIs.
//! - [`claims`]: [`claims::ExternalIdentity`] and the group rules.
//!
//! The server talks to providers only while serving a request (admin create/test,
//! sign-in); tests use a fake provider on 127.0.0.1.

pub mod claims;
pub mod fields;
pub mod http;
pub mod oidc;
pub mod presets;
pub mod signin;
pub mod social;
pub mod store;

use std::sync::Arc;

use http::HttpClient;
use oidc::Discovery;
use social::SocialEndpoints;

use crate::errors::ApiError;

/// Shared handles for talking to providers (part of `AppState`).
#[derive(Clone, Debug)]
pub struct Idp {
    pub http: HttpClient,
    pub discovery: Discovery,
    /// GitHub and Discord URLs: fixed in production, replaceable only in tests.
    social: Arc<SocialEndpoints>,
}

impl Idp {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: HttpClient::new()?,
            discovery: Discovery::default(),
            social: Arc::new(SocialEndpoints::production()?),
        })
    }

    /// Points GitHub and Discord at a fake server (tests only).
    #[cfg(test)]
    pub fn set_social_endpoints(&mut self, endpoints: SocialEndpoints) {
        self.social = Arc::new(endpoints);
    }

    /// OIDC discovery of `issuer` (cached unless `fresh`); failures are logged and
    /// answered as `provider_discovery_failed`.
    pub async fn discover(
        &self,
        issuer: &str,
        fresh: bool,
    ) -> Result<Arc<openidconnect::core::CoreProviderMetadata>, ApiError> {
        self.discovery
            .metadata(&self.http, issuer, fresh)
            .await
            .map_err(|err| {
                tracing::warn!(issuer, error = %format!("{err:#}"), "OIDC discovery failed");
                ApiError::ProviderDiscoveryFailed
            })
    }
}
