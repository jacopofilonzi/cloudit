//! [`ProviderFields`]: the admin-editable settings of a provider, and their validation.
//!
//! Create requests start from the preset's defaults, update requests from the stored
//! values; either way every rule is checked again on the result ([`ProviderFields::check`]),
//! so a field can't become inconsistent with the kind. Rules (errors are
//! `validation_failed` naming the field):
//!
//! - `id`: `[a-z0-9-]{2,32}`, immutable;
//! - `display_name`: 1..=100 characters; `icon`: a preset icon key or `generic`;
//! - `issuer_url`: only where the preset allows it (required for self-hosted kinds),
//!   `http(s)://` without query or fragment, stored as typed (the issuer must match the
//!   discovery document exactly, trailing slash included);
//! - `tenant`: Microsoft only and required there, `[A-Za-z0-9.-]{1,100}`, not
//!   `common`, `organizations` or `consumers`;
//! - `client_id` 1..=512 characters; `client_secret` 1..=4096 bytes;
//! - `scopes`: 1..=50 RFC 6749 scope tokens, normalized to single spaces; OIDC kinds
//!   must include `openid`;
//! - `groups_claim`, `admin_groups`, `allowed_groups`: only for kinds that support
//!   groups; claim names 1..=100 characters without whitespace; at most 100 groups of
//!   1..=256 characters each, trimmed and deduplicated;
//! - `position`: 0..=1_000_000.

use cloudit_core::protocol::{
    CreateProviderRequest, ProviderKind, ProviderProtocol, UpdateProviderRequest,
};

use super::presets::{self, MULTI_TENANT};
use crate::errors::ApiError;
use crate::validate;

const MAX_CLIENT_ID_CHARS: usize = 512;
const MAX_SECRET_BYTES: usize = 4096;
const MAX_SCOPES: usize = 50;
const MAX_GROUPS: usize = 100;
const MAX_GROUP_CHARS: usize = 256;
const MAX_POSITION: i32 = 1_000_000;

/// Editable settings of a provider (everything but id, kind and secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFields {
    pub display_name: String,
    pub icon: String,
    pub issuer_url: Option<String>,
    pub tenant: Option<String>,
    pub client_id: String,
    pub scopes: String,
    pub groups_claim: Option<String>,
    pub admin_groups: Vec<String>,
    pub allowed_groups: Vec<String>,
    pub enabled: bool,
    pub position: i32,
}

/// Checks a provider id.
pub fn id(raw: &str) -> Result<String, ApiError> {
    let ok = (2..=32).contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(raw.to_owned())
    } else {
        Err(ApiError::validation("id", "2 to 32 of a-z, 0-9 and -"))
    }
}

/// Checks a client secret.
pub fn client_secret(raw: &str) -> Result<&str, ApiError> {
    if raw.is_empty() || raw.len() > MAX_SECRET_BYTES {
        return Err(ApiError::validation(
            "client_secret",
            format!("1 to {MAX_SECRET_BYTES} bytes"),
        ));
    }
    Ok(raw)
}

/// `None` for an empty (or whitespace) string: "clear this value".
fn non_empty(raw: &str) -> Option<&str> {
    Some(raw.trim()).filter(|v| !v.is_empty())
}

fn issuer_url(raw: &str) -> Result<String, ApiError> {
    let invalid = |why: &str| ApiError::validation("issuer_url", why.to_owned());
    let value = raw.trim();
    let url = url::Url::parse(value).map_err(|_| invalid("not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(invalid("must be an http(s) URL with a host"));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(invalid("must not have a query, fragment or credentials"));
    }
    Ok(value.to_owned())
}

fn tenant(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    let shape = (1..=100).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !shape {
        return Err(ApiError::validation(
            "tenant",
            "a tenant id or domain (A-Z, 0-9, . and -)",
        ));
    }
    if MULTI_TENANT.iter().any(|m| m.eq_ignore_ascii_case(value)) {
        return Err(ApiError::validation(
            "tenant",
            "multi-tenant endpoints are not supported: use your directory's tenant id",
        ));
    }
    Ok(value.to_owned())
}

fn scopes(raw: &str) -> Result<String, ApiError> {
    // RFC 6749 scope-token: %x21 / %x23-5B / %x5D-7E.
    let token_ok = |t: &str| {
        t.bytes()
            .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
    };
    let tokens: Vec<&str> = raw.split_ascii_whitespace().collect();
    if tokens.is_empty() || tokens.len() > MAX_SCOPES || !tokens.iter().all(|t| token_ok(t)) {
        return Err(ApiError::validation(
            "scopes",
            format!("1 to {MAX_SCOPES} space-separated scopes"),
        ));
    }
    let mut unique: Vec<&str> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if !unique.contains(&token) {
            unique.push(token);
        }
    }
    Ok(unique.join(" "))
}

fn groups_claim(raw: &str) -> Result<String, ApiError> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > 100
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ApiError::validation(
            "groups_claim",
            "1 to 100 characters, no whitespace",
        ));
    }
    Ok(value.to_owned())
}

fn groups(list: &[String], field: &'static str) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let group = validate::text(raw, field, MAX_GROUP_CHARS)?;
        if !out.contains(&group) {
            out.push(group);
        }
    }
    if out.len() > MAX_GROUPS {
        return Err(ApiError::validation(
            field,
            format!("at most {MAX_GROUPS} groups"),
        ));
    }
    Ok(out)
}

impl ProviderFields {
    /// The effective OIDC issuer of a provider of `kind` with these fields.
    #[must_use]
    pub fn issuer(&self, kind: ProviderKind) -> Option<String> {
        let preset = presets::preset(kind);
        if preset.protocol != ProviderProtocol::Oidc {
            return None;
        }
        if preset.tenant_required {
            return self.tenant.as_deref().map(presets::microsoft_issuer);
        }
        self.issuer_url
            .clone()
            .or_else(|| preset.issuer.map(str::to_owned))
    }

    /// Fields of a new provider: request values over the preset's defaults.
    pub fn from_create(
        req: &CreateProviderRequest,
        default_position: i32,
    ) -> Result<Self, ApiError> {
        let preset = presets::preset(req.kind);
        let mut fields = Self {
            display_name: preset.display_name.to_owned(),
            icon: preset.icon.to_owned(),
            issuer_url: None,
            tenant: None,
            client_id: String::new(),
            scopes: preset.default_scopes.to_owned(),
            groups_claim: None,
            admin_groups: Vec::new(),
            allowed_groups: Vec::new(),
            enabled: req.enabled.unwrap_or(true),
            position: req.position.unwrap_or(default_position),
        };
        fields.apply(
            req.display_name.as_deref(),
            req.icon.as_deref(),
            req.issuer_url.as_deref(),
            req.tenant.as_deref(),
            Some(&req.client_id),
            req.scopes.as_deref(),
            req.groups_claim.as_deref(),
            req.admin_groups.as_deref(),
            req.allowed_groups.as_deref(),
        )?;
        fields.check(req.kind)?;
        Ok(fields)
    }

    /// Fields after an update request.
    pub fn with_update(
        &self,
        kind: ProviderKind,
        req: &UpdateProviderRequest,
    ) -> Result<Self, ApiError> {
        let mut fields = self.clone();
        if let Some(enabled) = req.enabled {
            fields.enabled = enabled;
        }
        if let Some(position) = req.position {
            fields.position = position;
        }
        fields.apply(
            req.display_name.as_deref(),
            req.icon.as_deref(),
            req.issuer_url.as_deref(),
            req.tenant.as_deref(),
            req.client_id.as_deref(),
            req.scopes.as_deref(),
            req.groups_claim.as_deref(),
            req.admin_groups.as_deref(),
            req.allowed_groups.as_deref(),
        )?;
        fields.check(kind)?;
        Ok(fields)
    }

    /// Normalizes and stores the given values (`Some("")` clears the nullable ones).
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        display_name: Option<&str>,
        icon: Option<&str>,
        issuer: Option<&str>,
        tenant_value: Option<&str>,
        client_id: Option<&str>,
        scope_list: Option<&str>,
        claim: Option<&str>,
        admin_groups: Option<&[String]>,
        allowed_groups: Option<&[String]>,
    ) -> Result<(), ApiError> {
        if let Some(name) = display_name {
            self.display_name = validate::display_name(name)?;
        }
        if let Some(icon) = icon {
            let icon = icon.trim();
            if !presets::is_icon(icon) {
                return Err(ApiError::validation("icon", "unknown icon"));
            }
            icon.clone_into(&mut self.icon);
        }
        if let Some(raw) = issuer {
            self.issuer_url = non_empty(raw).map(issuer_url).transpose()?;
        }
        if let Some(raw) = tenant_value {
            self.tenant = non_empty(raw).map(tenant).transpose()?;
        }
        if let Some(raw) = client_id {
            self.client_id = validate::text(raw, "client_id", MAX_CLIENT_ID_CHARS)?;
        }
        if let Some(raw) = scope_list {
            self.scopes = scopes(raw)?;
        }
        if let Some(raw) = claim {
            self.groups_claim = non_empty(raw).map(groups_claim).transpose()?;
        }
        if let Some(list) = admin_groups {
            self.admin_groups = groups(list, "admin_groups")?;
        }
        if let Some(list) = allowed_groups {
            self.allowed_groups = groups(list, "allowed_groups")?;
        }
        Ok(())
    }

    /// Rules that depend on the kind or on several fields.
    fn check(&self, kind: ProviderKind) -> Result<(), ApiError> {
        let preset = presets::preset(kind);
        match (
            &self.issuer_url,
            preset.issuer_editable,
            preset.issuer_required,
        ) {
            (Some(_), false, _) => {
                return Err(ApiError::validation(
                    "issuer_url",
                    "not configurable for this provider",
                ));
            }
            (None, _, true) => return Err(ApiError::validation("issuer_url", "required")),
            _ => {}
        }
        match (&self.tenant, preset.tenant_required) {
            (Some(_), false) => {
                return Err(ApiError::validation("tenant", "only for Microsoft"));
            }
            (None, true) => return Err(ApiError::validation("tenant", "required")),
            _ => {}
        }
        if preset.protocol == ProviderProtocol::Oidc
            && !self.scopes.split(' ').any(|s| s == "openid")
        {
            return Err(ApiError::validation("scopes", "must include openid"));
        }
        if !preset.supports_groups {
            for (field, set) in [
                ("groups_claim", self.groups_claim.is_some()),
                ("admin_groups", !self.admin_groups.is_empty()),
                ("allowed_groups", !self.allowed_groups.is_empty()),
            ] {
                if set {
                    return Err(ApiError::validation(field, "this provider has no groups"));
                }
            }
        }
        if !(0..=MAX_POSITION).contains(&self.position) {
            return Err(ApiError::validation(
                "position",
                format!("0 to {MAX_POSITION}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudit_core::protocol::Sensitive;

    fn create(kind: ProviderKind) -> CreateProviderRequest {
        CreateProviderRequest {
            id: "p".to_owned(),
            kind,
            display_name: None,
            icon: None,
            issuer_url: None,
            tenant: None,
            client_id: "client".to_owned(),
            client_secret: Sensitive::new("secret"),
            scopes: None,
            groups_claim: None,
            admin_groups: None,
            allowed_groups: None,
            enabled: None,
            position: None,
        }
    }

    fn field_of(err: ApiError) -> &'static str {
        match err {
            ApiError::Validation { field, .. } => field,
            other => panic!("not a validation error: {other:?}"),
        }
    }

    #[test]
    fn ids() {
        assert!(id("my-idp-2").is_ok());
        for bad in ["a", "UPPER", "with space", "x_y", &"a".repeat(33), "ünï"] {
            assert!(id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn defaults_come_from_the_preset() {
        let f = ProviderFields::from_create(&create(ProviderKind::Google), 3).unwrap();
        assert_eq!(f.display_name, "Google");
        assert_eq!(f.icon, "google");
        assert_eq!(f.scopes, "openid email profile");
        assert_eq!(f.position, 3);
        assert!(f.enabled);
        assert_eq!(
            f.issuer(ProviderKind::Google).as_deref(),
            Some("https://accounts.google.com")
        );
        let f = ProviderFields::from_create(&create(ProviderKind::Github), 0).unwrap();
        assert_eq!(f.scopes, "read:user user:email");
        assert_eq!(f.issuer(ProviderKind::Github), None);
    }

    #[test]
    fn issuer_rules() {
        let mut req = create(ProviderKind::Keycloak);
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "issuer_url"
        );
        req.issuer_url = Some("https://kc.example.com/realms/main".to_owned());
        let f = ProviderFields::from_create(&req, 0).unwrap();
        assert_eq!(
            f.issuer(ProviderKind::Keycloak).as_deref(),
            Some("https://kc.example.com/realms/main")
        );
        req.issuer_url = Some("https://kc.example.com/?x=1".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "issuer_url"
        );

        let mut req = create(ProviderKind::Google);
        req.issuer_url = Some("https://evil.example.com".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "issuer_url"
        );

        let mut req = create(ProviderKind::Gitlab);
        let f = ProviderFields::from_create(&req, 0).unwrap();
        assert_eq!(
            f.issuer(ProviderKind::Gitlab).as_deref(),
            Some("https://gitlab.com")
        );
        req.issuer_url = Some("https://git.example.com".to_owned());
        let f = ProviderFields::from_create(&req, 0).unwrap();
        assert_eq!(
            f.issuer(ProviderKind::Gitlab).as_deref(),
            Some("https://git.example.com")
        );
    }

    #[test]
    fn microsoft_tenant_rules() {
        let mut req = create(ProviderKind::Microsoft);
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "tenant"
        );
        for multi in ["common", "Organizations", "consumers"] {
            req.tenant = Some(multi.to_owned());
            assert_eq!(
                field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
                "tenant"
            );
        }
        req.tenant = Some("contoso.onmicrosoft.com".to_owned());
        let f = ProviderFields::from_create(&req, 0).unwrap();
        assert_eq!(
            f.issuer(ProviderKind::Microsoft).as_deref(),
            Some("https://login.microsoftonline.com/contoso.onmicrosoft.com/v2.0")
        );
        let mut req = create(ProviderKind::Google);
        req.tenant = Some("x".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "tenant"
        );
    }

    #[test]
    fn scopes_and_groups() {
        let mut req = create(ProviderKind::Oidc);
        req.issuer_url = Some("https://idp.example.com".to_owned());
        req.scopes = Some("  openid   email openid groups ".to_owned());
        req.admin_groups = Some(vec![" admins ".to_owned(), "admins".to_owned()]);
        let f = ProviderFields::from_create(&req, 0).unwrap();
        assert_eq!(f.scopes, "openid email groups");
        assert_eq!(f.admin_groups, ["admins"]);
        req.scopes = Some("email profile".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "scopes"
        );
        req.scopes = Some("openid bad\"scope".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "scopes"
        );

        let mut req = create(ProviderKind::Github);
        req.allowed_groups = Some(vec!["staff".to_owned()]);
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "allowed_groups"
        );
        let mut req = create(ProviderKind::Discord);
        req.groups_claim = Some("roles".to_owned());
        assert_eq!(
            field_of(ProviderFields::from_create(&req, 0).unwrap_err()),
            "groups_claim"
        );
    }

    #[test]
    fn updates_overlay_and_clear() {
        let mut req = create(ProviderKind::Gitlab);
        req.issuer_url = Some("https://git.example.com".to_owned());
        req.groups_claim = Some("groups_direct".to_owned());
        let f = ProviderFields::from_create(&req, 0).unwrap();
        let update = UpdateProviderRequest {
            display_name: Some("Company GitLab".to_owned()),
            icon: None,
            issuer_url: Some(String::new()),
            tenant: None,
            client_id: None,
            client_secret: None,
            scopes: None,
            groups_claim: Some(String::new()),
            admin_groups: None,
            allowed_groups: None,
            enabled: Some(false),
            position: Some(-1),
        };
        assert_eq!(
            field_of(f.with_update(ProviderKind::Gitlab, &update).unwrap_err()),
            "position"
        );
        let update = UpdateProviderRequest {
            position: Some(7),
            ..update
        };
        let g = f.with_update(ProviderKind::Gitlab, &update).unwrap();
        assert_eq!(g.display_name, "Company GitLab");
        assert_eq!(g.issuer_url, None);
        assert_eq!(g.groups_claim, None);
        assert!(!g.enabled);
        assert_eq!(g.position, 7);
        assert_eq!(g.client_id, "client");
    }
}
