//! The provider side of a sign-in: the authorization URL, the code exchange and the
//! resulting [`ExternalIdentity`].
//!
//! - Authorization: `response_type=code`, `client_id`, `redirect_uri`, `scope`,
//!   `state`, PKCE S256 `code_challenge` and, for OIDC, `nonce`.
//! - Token: `POST` form (`authorization_code`, `code`, `redirect_uri`,
//!   `code_verifier`); the client authenticates with HTTP Basic (RFC 6749 form-encoded
//!   id and secret) unless an OIDC provider only lists `client_secret_post`; GitHub and
//!   Discord get the secret in the body.
//! - OIDC: the ID token is verified by `openidconnect` (signature against the JWKS,
//!   issuer = the configured one, audience = our client id, expiry, nonce); algorithms
//!   RS/PS/ES/EdDSA, or HS* with the client secret. An unknown key id refetches the
//!   JWKS once. Groups come from the configured claim of the verified token's payload.
//!   If the token lacks the email, or lacks the groups claim while the provider uses
//!   groups, the user-info endpoint fills them in (its `sub` must match).
//! - OAuth2 kinds: see [`super::social`].
//!
//! Every provider failure is logged with its cause and answered as `provider_error`.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cloudit_core::protocol::ProviderProtocol;
use openidconnect::core::{
    CoreClientAuthMethod, CoreIdToken, CoreIdTokenVerifier, CoreJwsSigningAlgorithm,
    CoreProviderMetadata,
};
use openidconnect::{ClaimsVerificationError, ClientId, ClientSecret, IssuerUrl, Nonce};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::Idp;
use super::claims::{self, ExternalIdentity};

use super::social;
use super::store::Provider;
use crate::config::Secret;
use crate::errors::ApiError;

/// ID token algorithms accepted (never `none`).
const ALGORITHMS: [CoreJwsSigningAlgorithm; 13] = [
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
    CoreJwsSigningAlgorithm::RsaSsaPssSha256,
    CoreJwsSigningAlgorithm::RsaSsaPssSha384,
    CoreJwsSigningAlgorithm::RsaSsaPssSha512,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EcdsaP384Sha384,
    CoreJwsSigningAlgorithm::EcdsaP521Sha512,
    CoreJwsSigningAlgorithm::EdDsa,
    CoreJwsSigningAlgorithm::HmacSha256,
    CoreJwsSigningAlgorithm::HmacSha384,
    CoreJwsSigningAlgorithm::HmacSha512,
];

/// The values of one authorization request.
pub struct AuthRequest<'a> {
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    /// base64url SHA-256 of the PKCE verifier.
    pub code_challenge: &'a str,
}

/// Logs a provider failure and returns `provider_error`.
fn provider_error(provider: &Provider, what: &str, cause: impl std::fmt::Display) -> ApiError {
    tracing::warn!(provider = %provider.id, error = %cause, "{what}");
    ApiError::ProviderError
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    id_token: Option<String>,
    error: Option<String>,
}

/// RFC 6749 §2.3.1: id and secret are form-encoded before Basic encoding.
fn basic_auth_value(client_id: &str, secret: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", enc(client_id), enc(secret)))
}

/// Groups claim from the (already verified) ID token's payload.
fn payload_claims(raw: &str) -> Option<Value> {
    let payload = raw.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl Idp {
    async fn oidc_metadata(
        &self,
        provider: &Provider,
    ) -> Result<std::sync::Arc<CoreProviderMetadata>, ApiError> {
        let issuer = provider.issuer().ok_or(ApiError::ProviderError)?;
        self.discover(&issuer, false).await
    }

    fn oauth2_endpoints(&self, provider: &Provider) -> Result<&social::OAuth2Endpoints, ApiError> {
        self.social.of(provider.kind).ok_or_else(|| {
            provider_error(provider, "no endpoints for kind", provider.kind.as_str())
        })
    }

    /// The provider's authorization URL for `req`.
    pub async fn authorize_url(
        &self,
        provider: &Provider,
        req: &AuthRequest<'_>,
    ) -> Result<String, ApiError> {
        let (mut url, oidc) = match provider.protocol() {
            ProviderProtocol::Oidc => {
                let metadata = self.oidc_metadata(provider).await?;
                (metadata.authorization_endpoint().url().clone(), true)
            }
            ProviderProtocol::Oauth2 => (self.oauth2_endpoints(provider)?.authorize.clone(), false),
        };
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &provider.fields.client_id)
                .append_pair("redirect_uri", req.redirect_uri)
                .append_pair("scope", &provider.fields.scopes)
                .append_pair("state", req.state)
                .append_pair("code_challenge", req.code_challenge)
                .append_pair("code_challenge_method", "S256");
            if oidc {
                query.append_pair("nonce", req.nonce);
            }
        }
        Ok(url.into())
    }

    /// Exchanges `code` and returns who signed in (see the module docs).
    pub async fn sign_in(
        &self,
        provider: &Provider,
        client_secret: &Secret,
        code: &str,
        verifier: &str,
        nonce: &str,
        redirect_uri: &str,
    ) -> Result<ExternalIdentity, ApiError> {
        match provider.protocol() {
            ProviderProtocol::Oidc => {
                let metadata = self.oidc_metadata(provider).await?;
                let token_url = metadata
                    .token_endpoint()
                    .map(|t| t.url().clone())
                    .ok_or_else(|| provider_error(provider, "no token endpoint", "discovery"))?;
                let post_only = metadata
                    .token_endpoint_auth_methods_supported()
                    .is_some_and(|methods| {
                        methods.contains(&CoreClientAuthMethod::ClientSecretPost)
                            && !methods.contains(&CoreClientAuthMethod::ClientSecretBasic)
                    });
                let tokens = self
                    .exchange(
                        provider,
                        &token_url,
                        client_secret,
                        code,
                        verifier,
                        redirect_uri,
                        !post_only,
                    )
                    .await?;
                let raw = tokens.id_token.ok_or_else(|| {
                    provider_error(provider, "token response without id_token", "oidc")
                })?;
                let access = tokens.access_token.unwrap_or_default();
                self.oidc_identity(provider, metadata, client_secret, &raw, nonce, &access)
                    .await
            }
            ProviderProtocol::Oauth2 => {
                let endpoints = self.oauth2_endpoints(provider)?.clone();
                let tokens = self
                    .exchange(
                        provider,
                        &endpoints.token,
                        client_secret,
                        code,
                        verifier,
                        redirect_uri,
                        false,
                    )
                    .await?;
                let access = tokens.access_token.ok_or_else(|| {
                    provider_error(provider, "token response without access_token", "oauth2")
                })?;
                social::fetch_identity(&self.http, provider.kind, &endpoints, &access)
                    .await
                    .map_err(|e| {
                        provider_error(provider, "reading the user from the provider API", e)
                    })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn exchange(
        &self,
        provider: &Provider,
        token_url: &Url,
        client_secret: &Secret,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        basic: bool,
    ) -> Result<TokenResponse, ApiError> {
        let client_id = provider.fields.client_id.as_str();
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ];
        let auth = if basic {
            Some(basic_auth_value(client_id, client_secret.expose()))
        } else {
            form.push(("client_id", client_id));
            form.push(("client_secret", client_secret.expose()));
            None
        };
        let response = self
            .http
            .post_form_auth(token_url, &form, auth.as_deref())
            .await
            .map_err(|e| provider_error(provider, "token request failed", e))?;
        let parsed: Result<TokenResponse, _> = serde_json::from_slice(response.body());
        match parsed {
            Ok(tokens) if response.status().is_success() && tokens.error.is_none() => Ok(tokens),
            Ok(tokens) => Err(provider_error(
                provider,
                "token request refused",
                format!(
                    "status {}, error {}",
                    response.status(),
                    tokens.error.as_deref().unwrap_or("-")
                ),
            )),
            Err(err) => Err(provider_error(
                provider,
                "unreadable token response",
                format!("status {}: {err}", response.status()),
            )),
        }
    }

    async fn oidc_identity(
        &self,
        provider: &Provider,
        metadata: std::sync::Arc<CoreProviderMetadata>,
        client_secret: &Secret,
        raw: &str,
        nonce: &str,
        access_token: &str,
    ) -> Result<ExternalIdentity, ApiError> {
        let issuer = provider.issuer().ok_or(ApiError::ProviderError)?;
        let id_token: CoreIdToken = raw
            .parse()
            .map_err(|e| provider_error(provider, "malformed ID token", e))?;
        let nonce = Nonce::new(nonce.to_owned());
        let issuer_url = IssuerUrl::new(issuer.clone())
            .map_err(|e| provider_error(provider, "invalid issuer", e))?;
        let verify = |metadata: &CoreProviderMetadata| {
            let verifier = CoreIdTokenVerifier::new_confidential_client(
                ClientId::new(provider.fields.client_id.clone()),
                ClientSecret::new(client_secret.expose().to_owned()),
                issuer_url.clone(),
                metadata.jwks().clone(),
            )
            .set_allowed_algs(ALGORITHMS);
            id_token.claims(&verifier, &nonce).cloned()
        };
        let claims = match verify(&metadata) {
            Err(ClaimsVerificationError::SignatureVerification(
                openidconnect::SignatureVerificationError::NoMatchingKey,
            )) => {
                // Key rotation: refetch the keys once.
                let refreshed = self
                    .discovery
                    .refresh_jwks(&self.http, &issuer, &metadata)
                    .await
                    .map_err(|e| {
                        provider_error(provider, "refreshing provider keys", format!("{e:#}"))
                    })?;
                verify(&refreshed)
            }
            other => other,
        }
        .map_err(|e| provider_error(provider, "ID token rejected", super::oidc::error_chain(&e)))?;

        let payload = payload_claims(raw).unwrap_or(Value::Null);
        let mut groups = claims::groups_from(payload.get(provider.groups_claim()));
        let mut email = claims.email().map(|e| e.as_str().to_owned());
        let mut email_verified = claims.email_verified().unwrap_or(false);
        let mut names: Vec<Option<String>> = vec![
            claims
                .name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str().to_owned()),
            claims.preferred_username().map(|u| u.as_str().to_owned()),
        ];

        let uses_groups =
            !provider.fields.admin_groups.is_empty() || !provider.fields.allowed_groups.is_empty();
        let need_userinfo = email.is_none() || (groups.is_none() && uses_groups);
        if need_userinfo && let Some(endpoint) = metadata.userinfo_endpoint() {
            let info: Value = self
                .http
                .get_json(endpoint.url(), Some(access_token))
                .await
                .map_err(|e| provider_error(provider, "user info request failed", e))?;
            if info.get("sub").and_then(Value::as_str) != Some(claims.subject().as_str()) {
                return Err(provider_error(
                    provider,
                    "user info for another subject",
                    "sub mismatch",
                ));
            }
            if email.is_none() {
                email = info.get("email").and_then(Value::as_str).map(str::to_owned);
                email_verified = claims::bool_claim(info.get("email_verified"));
            }
            if groups.is_none() {
                groups = claims::groups_from(info.get(provider.groups_claim()));
            }
            names.push(info.get("name").and_then(Value::as_str).map(str::to_owned));
            names.push(
                info.get("preferred_username")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let names: Vec<Option<&str>> = names.iter().map(Option::as_deref).collect();
        Ok(ExternalIdentity::new(
            claims.subject().as_str().to_owned(),
            email.as_deref(),
            email_verified,
            &names,
            groups,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth_is_form_encoded() {
        let value = basic_auth_value("my id", "p@ss:w/rd");
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, "my+id:p%40ss%3Aw%2Frd");
    }

    #[test]
    fn reads_the_payload() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"groups":["a"]}"#);
        let claims = payload_claims(&format!("h.{payload}.s")).unwrap();
        assert_eq!(claims["groups"][0], "a");
        assert!(payload_claims("nodots").is_none());
    }
}
