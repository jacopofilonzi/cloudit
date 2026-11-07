//! OIDC discovery with an in-memory cache.
//!
//! `openidconnect` fetches `<issuer>/.well-known/openid-configuration`, checks that the
//! document's `issuer` equals the configured one exactly, then fetches the JWKS. The
//! result is cached per issuer for [`TTL`]. This is a cache, not state: any instance
//! can refetch it, so the server stays stateless. Failures are not cached. When an ID
//! token is signed with a key id the cached JWKS doesn't have (key rotation), the JWKS
//! is refetched once ([`Discovery::refresh_jwks`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use openidconnect::IssuerUrl;
use openidconnect::core::{CoreJsonWebKeySet, CoreProviderMetadata};

use super::http::HttpClient;

/// How long discovery documents and keys are reused.
pub const TTL: Duration = Duration::from_secs(3600);

struct Entry {
    metadata: Arc<CoreProviderMetadata>,
    fetched: Instant,
}

/// The discovery cache (cheap to clone, shared by all requests of an instance).
#[derive(Clone, Default)]
pub struct Discovery {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Discovery")
    }
}

impl Discovery {
    fn cached(&self, issuer: &str) -> Option<Arc<CoreProviderMetadata>> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(issuer)
            .filter(|e| e.fetched.elapsed() < TTL)
            .map(|e| Arc::clone(&e.metadata))
    }

    fn store(&self, issuer: &str, metadata: Arc<CoreProviderMetadata>) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                issuer.to_owned(),
                Entry {
                    metadata,
                    fetched: Instant::now(),
                },
            );
        }
    }

    /// The provider metadata (with its JWKS) of `issuer`: from the cache unless
    /// `fresh`. The error is a description for the logs.
    pub async fn metadata(
        &self,
        http: &HttpClient,
        issuer: &str,
        fresh: bool,
    ) -> anyhow::Result<Arc<CoreProviderMetadata>> {
        if !fresh && let Some(metadata) = self.cached(issuer) {
            return Ok(metadata);
        }
        let issuer_url = IssuerUrl::new(issuer.to_owned())
            .map_err(|e| anyhow::anyhow!("invalid issuer URL: {e}"))?;
        let adapter = http.adapter();
        let metadata = CoreProviderMetadata::discover_async(issuer_url, &adapter)
            .await
            .map_err(|e| anyhow::anyhow!("discovery of {issuer} failed: {}", error_chain(&e)))?;
        let metadata = Arc::new(metadata);
        self.store(issuer, Arc::clone(&metadata));
        Ok(metadata)
    }

    /// Refetches the JWKS of `issuer` (unknown key id in an ID token) and updates the
    /// cache; returns the updated metadata.
    pub async fn refresh_jwks(
        &self,
        http: &HttpClient,
        issuer: &str,
        metadata: &CoreProviderMetadata,
    ) -> anyhow::Result<Arc<CoreProviderMetadata>> {
        let adapter = http.adapter();
        let jwks = CoreJsonWebKeySet::fetch_async(metadata.jwks_uri(), &adapter)
            .await
            .map_err(|e| anyhow::anyhow!("fetching the keys of {issuer}: {}", error_chain(&e)))?;
        let updated = Arc::new(metadata.clone().set_jwks(jwks));
        self.store(issuer, Arc::clone(&updated));
        Ok(updated)
    }
}

/// `err` and its sources, `: `-separated (for logs).
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
