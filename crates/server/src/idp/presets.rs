//! The provider presets: one static entry per [`ProviderKind`] with its protocol,
//! issuer rules, default scopes and whether it can send groups.
//!
//! - OIDC with a fixed issuer: Google; Microsoft Entra ID with a required tenant
//!   (`https://login.microsoftonline.com/{tenant}/v2.0`). The multi-tenant endpoints
//!   `common`, `organizations` and `consumers` are refused: their ID tokens carry the
//!   user's own tenant as issuer, which can't be checked against a configured one.
//! - OIDC with a default issuer the admin may override: GitLab (self-hosted GitLab).
//! - OIDC, issuer required: Authentik, Keycloak, Authelia, Zitadel, Pocket ID and
//!   generic OIDC.
//! - OAuth2 with a provider-specific user API: GitHub, Discord (see `idp::social`).
//!
//! Apple is not offered: its client secret is a short-lived signed JWT, which needs key
//! management of its own (future work).

use cloudit_core::protocol::{NewAccounts, ProviderKind, ProviderPreset, ProviderProtocol};

/// Scopes of OIDC presets.
const OIDC_SCOPES: &str = "openid email profile";

/// Static description of a kind (see the module docs).
#[derive(Debug)]
pub struct Preset {
    pub kind: ProviderKind,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub protocol: ProviderProtocol,
    /// Default (or fixed) issuer.
    pub issuer: Option<&'static str>,
    pub issuer_editable: bool,
    pub issuer_required: bool,
    pub tenant_required: bool,
    pub default_scopes: &'static str,
    pub supports_groups: bool,
}

const fn oidc_self_hosted(
    kind: ProviderKind,
    display_name: &'static str,
    icon: &'static str,
) -> Preset {
    Preset {
        kind,
        display_name,
        icon,
        protocol: ProviderProtocol::Oidc,
        issuer: None,
        issuer_editable: true,
        issuer_required: true,
        tenant_required: false,
        default_scopes: OIDC_SCOPES,
        supports_groups: true,
    }
}

/// All presets, in [`ProviderKind::ALL`] order.
pub static PRESETS: [Preset; 11] = [
    Preset {
        kind: ProviderKind::Google,
        display_name: "Google",
        icon: "google",
        protocol: ProviderProtocol::Oidc,
        issuer: Some("https://accounts.google.com"),
        issuer_editable: false,
        issuer_required: false,
        tenant_required: false,
        default_scopes: OIDC_SCOPES,
        supports_groups: false,
    },
    Preset {
        kind: ProviderKind::Microsoft,
        display_name: "Microsoft",
        icon: "microsoft",
        protocol: ProviderProtocol::Oidc,
        issuer: None,
        issuer_editable: false,
        issuer_required: false,
        tenant_required: true,
        default_scopes: OIDC_SCOPES,
        supports_groups: true,
    },
    Preset {
        kind: ProviderKind::Gitlab,
        display_name: "GitLab",
        icon: "gitlab",
        protocol: ProviderProtocol::Oidc,
        issuer: Some("https://gitlab.com"),
        issuer_editable: true,
        issuer_required: false,
        tenant_required: false,
        default_scopes: OIDC_SCOPES,
        supports_groups: true,
    },
    oidc_self_hosted(ProviderKind::Authentik, "authentik", "authentik"),
    oidc_self_hosted(ProviderKind::Keycloak, "Keycloak", "keycloak"),
    oidc_self_hosted(ProviderKind::Authelia, "Authelia", "authelia"),
    oidc_self_hosted(ProviderKind::Zitadel, "ZITADEL", "zitadel"),
    oidc_self_hosted(ProviderKind::Pocketid, "Pocket ID", "pocketid"),
    oidc_self_hosted(ProviderKind::Oidc, "OpenID Connect", "generic"),
    Preset {
        kind: ProviderKind::Github,
        display_name: "GitHub",
        icon: "github",
        protocol: ProviderProtocol::Oauth2,
        issuer: None,
        issuer_editable: false,
        issuer_required: false,
        tenant_required: false,
        default_scopes: "read:user user:email",
        supports_groups: false,
    },
    Preset {
        kind: ProviderKind::Discord,
        display_name: "Discord",
        icon: "discord",
        protocol: ProviderProtocol::Oauth2,
        issuer: None,
        issuer_editable: false,
        issuer_required: false,
        tenant_required: false,
        default_scopes: "identify email",
        supports_groups: false,
    },
];

/// The preset of `kind`.
#[must_use]
pub fn preset(kind: ProviderKind) -> &'static Preset {
    // PRESETS has one entry per kind (checked by a test); fall back to generic OIDC
    // rather than panicking.
    PRESETS
        .iter()
        .find(|p| p.kind == kind)
        .unwrap_or(&PRESETS[8])
}

/// Accepted icon keys: every preset's icon (includes `generic`).
pub fn is_icon(name: &str) -> bool {
    PRESETS.iter().any(|p| p.icon == name)
}

/// Tenants that are not a single directory (see the module docs).
pub const MULTI_TENANT: [&str; 3] = ["common", "organizations", "consumers"];

/// Microsoft issuer of a single tenant.
#[must_use]
pub fn microsoft_issuer(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant}/v2.0")
}

impl Preset {
    #[must_use]
    pub fn to_api(&self) -> ProviderPreset {
        ProviderPreset {
            kind: self.kind,
            display_name: self.display_name.to_owned(),
            icon: self.icon.to_owned(),
            protocol: self.protocol,
            issuer_url: self.issuer.map(str::to_owned),
            issuer_editable: self.issuer_editable,
            issuer_required: self.issuer_required,
            tenant_required: self.tenant_required,
            default_scopes: self.default_scopes.to_owned(),
            supports_groups: self.supports_groups,
            new_accounts: NewAccounts::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_preset_per_kind_in_order() {
        for (preset, kind) in PRESETS.iter().zip(ProviderKind::ALL) {
            assert_eq!(preset.kind, kind);
            assert_eq!(super::preset(kind).kind, kind);
            assert!(is_icon(preset.icon));
            if preset.protocol == ProviderProtocol::Oidc {
                assert!(preset.default_scopes.split(' ').any(|s| s == "openid"));
            }
        }
        assert!(is_icon("generic"));
        assert!(!is_icon("apple"));
    }
}
