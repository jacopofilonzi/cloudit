//! Identity providers (OIDC / OAuth2) configured by admins
//! (`/api/v1/admin/providers*`).

use serde::{Deserialize, Serialize};

use super::{ErrorCode, NewAccounts, Sensitive};

/// Kind of identity provider: decides the protocol and the preset defaults. Stored
/// with each provider and never changed afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ProviderKind {
    /// OIDC, issuer `https://accounts.google.com`.
    Google,
    /// OIDC, Microsoft Entra ID single tenant: issuer
    /// `https://login.microsoftonline.com/{tenant}/v2.0`.
    Microsoft,
    /// OIDC, issuer `https://gitlab.com` or a self-hosted GitLab.
    Gitlab,
    /// OIDC, self-hosted (issuer required).
    Authentik,
    Keycloak,
    Authelia,
    Zitadel,
    Pocketid,
    /// Any other OIDC provider (issuer required).
    Oidc,
    /// OAuth2 (no OIDC): GitHub.
    Github,
    /// OAuth2 (no OIDC): Discord.
    Discord,
}

impl ProviderKind {
    /// Every kind, in the order the presets are listed.
    pub const ALL: [Self; 11] = [
        Self::Google,
        Self::Microsoft,
        Self::Gitlab,
        Self::Authentik,
        Self::Keycloak,
        Self::Authelia,
        Self::Zitadel,
        Self::Pocketid,
        Self::Oidc,
        Self::Github,
        Self::Discord,
    ];

    /// The stored and serialized name (`google`, `pocketid`…).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Gitlab => "gitlab",
            Self::Authentik => "authentik",
            Self::Keycloak => "keycloak",
            Self::Authelia => "authelia",
            Self::Zitadel => "zitadel",
            Self::Pocketid => "pocketid",
            Self::Oidc => "oidc",
            Self::Github => "github",
            Self::Discord => "discord",
        }
    }

    /// Parses [`ProviderKind::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// How cloudit talks to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ProviderProtocol {
    /// OpenID Connect: discovery, signed ID token, optional groups claim.
    Oidc,
    /// Plain OAuth2 with a provider-specific user API (GitHub, Discord); no groups.
    Oauth2,
}

/// One entry of `GET /api/v1/admin/providers/presets`: what the admin form needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ProviderPreset {
    pub kind: ProviderKind,
    pub display_name: String,
    /// Icon key (`google`, `github`… or `generic`).
    pub icon: String,
    pub protocol: ProviderProtocol,
    /// Default issuer (fixed when `issuer_editable` is false), if the kind has one.
    pub issuer_url: Option<String>,
    /// The admin may set `issuer_url` (self-hosted providers, GitLab).
    pub issuer_editable: bool,
    /// `issuer_url` must be set.
    pub issuer_required: bool,
    /// `tenant` must be set (Microsoft).
    pub tenant_required: bool,
    /// Space-separated scopes used when none are given.
    pub default_scopes: String,
    /// The provider can send groups (`groups_claim`, `admin_groups`,
    /// `allowed_groups` are accepted).
    pub supports_groups: bool,
    /// `new_accounts` of a provider created without one (`approval`).
    pub new_accounts: NewAccounts,
}

/// A configured provider as admins see it (`GET /api/v1/admin/providers`). The client
/// secret is never returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ProviderInfo {
    /// Durable id (`[a-z0-9-]{2,32}`), part of the callback URL; never reused.
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    pub icon: String,
    /// The configured issuer (`null` for kinds with a fixed or tenant-derived issuer).
    pub issuer_url: Option<String>,
    /// Microsoft tenant id or domain.
    pub tenant: Option<String>,
    pub client_id: String,
    /// Always `true`: the secret is stored encrypted and never shown.
    pub has_secret: bool,
    /// Space-separated.
    pub scopes: String,
    /// Claim holding the groups; `null` means `groups`.
    pub groups_claim: Option<String>,
    /// Members of any of these groups are admins (recomputed at each sign-in through
    /// this provider).
    pub admin_groups: Vec<String>,
    /// If not empty, only members of one of these groups may sign in.
    pub allowed_groups: Vec<String>,
    /// Accounts created through this provider without an invite: active at once
    /// (`auto`) or pending until an admin approves them (`approval`). An OIDC account
    /// that is in one of `admin_groups` is always approved.
    pub new_accounts: NewAccounts,
    pub enabled: bool,
    /// Display order on the login page (ascending).
    pub position: i32,
    /// The redirect URI to register at the provider:
    /// `<public URL>/api/v1/auth/oauth/<id>/callback`; `null` while no public URL is
    /// configured.
    pub callback_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// `POST /api/v1/admin/providers`. Omitted optional fields take the preset's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct CreateProviderRequest {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub display_name: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub icon: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub issuer_url: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub tenant: Option<String>,
    pub client_id: String,
    pub client_secret: Sensitive,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub scopes: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub groups_claim: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub admin_groups: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    /// Default `approval`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub new_accounts: Option<NewAccounts>,
    /// Default `true`.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    /// Default: after the last provider.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub position: Option<i32>,
}

/// `PATCH /api/v1/admin/providers/{id}`: omitted fields stay unchanged; `id` and
/// `kind` can't change. For `issuer_url`, `tenant` and `groups_claim` an empty string
/// clears the value; an omitted `client_secret` keeps the stored one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct UpdateProviderRequest {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub display_name: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub icon: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub issuer_url: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub tenant: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub client_id: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub client_secret: Option<Sensitive>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub scopes: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub groups_claim: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub admin_groups: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub new_accounts: Option<NewAccounts>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub position: Option<i32>,
}

/// `POST /api/v1/admin/providers/{id}/test`: read-only check of the provider's
/// metadata (OIDC discovery and keys) and of the stored secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ProviderTestResult {
    pub ok: bool,
    /// Why it failed (`provider_discovery_failed`, `provider_error`); `null` when ok.
    pub code: Option<ErrorCode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_names_match_serde() {
        for kind in ProviderKind::ALL {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
            assert_eq!(ProviderKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ProviderKind::parse("apple"), None);
    }
}
