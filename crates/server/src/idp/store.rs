//! The `auth_providers` table: [`Provider`] (a decoded row) and its queries.
//!
//! The client secret is encrypted with [`SecretBox`] bound to
//! `auth_providers:<id>:client_secret`, decrypted only to talk to the provider and
//! never returned by the API. Deleting is soft: `deleted_at` is set and the provider is
//! disabled; identities keep referencing it and the id is never reused.

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::{AuthProvider, ProviderInfo, ProviderKind, ProviderProtocol};
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _, QuerySelect as _,
};

use super::fields::ProviderFields;
use super::presets::{self, Preset};
use crate::config::Secret;
use crate::crypto::SecretBox;
use crate::db::entities::auth_providers;
use crate::errors::ApiError;
use crate::time::rfc3339;

/// A provider row with its JSON columns decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub id: String,
    pub kind: ProviderKind,
    pub fields: ProviderFields,
    secret_enc: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Associated data of a provider's encrypted client secret.
fn secret_aad(id: &str) -> Vec<u8> {
    format!("auth_providers:{id}:client_secret").into_bytes()
}

fn groups_json(groups: &[String]) -> anyhow::Result<String> {
    serde_json::to_string(groups).context("encoding groups")
}

impl Provider {
    fn from_model(m: auth_providers::Model) -> anyhow::Result<Self> {
        let kind = ProviderKind::parse(&m.kind)
            .with_context(|| format!("provider {} has unknown kind {:?}", m.id, m.kind))?;
        let groups = |text: &str, column: &str| -> anyhow::Result<Vec<String>> {
            serde_json::from_str(text)
                .with_context(|| format!("provider {} has an unreadable {column}", m.id))
        };
        Ok(Self {
            kind,
            fields: ProviderFields {
                display_name: m.display_name,
                icon: m.icon,
                issuer_url: m.issuer_url,
                tenant: m.tenant,
                client_id: m.client_id,
                scopes: m.scopes,
                groups_claim: m.groups_claim,
                admin_groups: groups(&m.admin_groups, "admin_groups")?,
                allowed_groups: groups(&m.allowed_groups, "allowed_groups")?,
                enabled: m.enabled,
                position: m.position,
            },
            secret_enc: m.client_secret_enc,
            created_at: m.created_at,
            updated_at: m.updated_at,
            deleted_at: m.deleted_at,
            id: m.id,
        })
    }

    #[must_use]
    pub fn preset(&self) -> &'static Preset {
        presets::preset(self.kind)
    }

    #[must_use]
    pub fn protocol(&self) -> ProviderProtocol {
        self.preset().protocol
    }

    /// The OIDC issuer to discover and to expect in ID tokens; `None` for OAuth2 kinds.
    #[must_use]
    pub fn issuer(&self) -> Option<String> {
        self.fields.issuer(self.kind)
    }

    /// Claim holding the groups (default `groups`).
    #[must_use]
    pub fn groups_claim(&self) -> &str {
        self.fields.groups_claim.as_deref().unwrap_or("groups")
    }

    /// Usable for sign-in: enabled and not deleted.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.fields.enabled && self.deleted_at.is_none()
    }

    /// The decrypted client secret.
    pub fn client_secret(&self, secrets: &SecretBox) -> anyhow::Result<Secret> {
        secrets
            .decrypt_string(&secret_aad(&self.id), &self.secret_enc)
            .map(Secret::new)
            .with_context(|| format!("decrypting the client secret of provider {}", self.id))
    }

    /// The admin view. `public_url` is the effective public URL (for the callback).
    #[must_use]
    pub fn info(&self, public_url: Option<&str>) -> ProviderInfo {
        let f = &self.fields;
        ProviderInfo {
            id: self.id.clone(),
            kind: self.kind,
            display_name: f.display_name.clone(),
            icon: f.icon.clone(),
            issuer_url: f.issuer_url.clone(),
            tenant: f.tenant.clone(),
            client_id: f.client_id.clone(),
            has_secret: !self.secret_enc.is_empty(),
            scopes: f.scopes.clone(),
            groups_claim: f.groups_claim.clone(),
            admin_groups: f.admin_groups.clone(),
            allowed_groups: f.allowed_groups.clone(),
            enabled: f.enabled,
            position: f.position,
            callback_url: public_url.map(|base| callback_url(base, &self.id)),
            created_at: rfc3339(self.created_at),
            updated_at: rfc3339(self.updated_at),
        }
    }

    /// The login-page view.
    #[must_use]
    pub fn public(&self) -> AuthProvider {
        AuthProvider {
            id: self.id.clone(),
            kind: self.kind,
            display_name: self.fields.display_name.clone(),
            icon: self.fields.icon.clone(),
        }
    }
}

/// The redirect URI of provider `id` under the public URL `base`.
#[must_use]
pub fn callback_url(base: &str, id: &str) -> String {
    format!("{base}/api/v1/auth/oauth/{id}/callback")
}

fn models_to_providers(models: Vec<auth_providers::Model>) -> anyhow::Result<Vec<Provider>> {
    models.into_iter().map(Provider::from_model).collect()
}

/// Non-deleted providers in display order (`enabled_only`: the login page's list).
pub async fn list(db: &impl ConnectionTrait, enabled_only: bool) -> anyhow::Result<Vec<Provider>> {
    let mut query =
        auth_providers::Entity::find().filter(auth_providers::Column::DeletedAt.is_null());
    if enabled_only {
        query = query.filter(auth_providers::Column::Enabled.eq(true));
    }
    let models = query
        .order_by_asc(auth_providers::Column::Position)
        .order_by_asc(auth_providers::Column::Id)
        .all(db)
        .await
        .context("listing identity providers")?;
    models_to_providers(models)
}

/// Every provider, deleted ones included (identities may reference them).
pub async fn list_all(db: &impl ConnectionTrait) -> anyhow::Result<Vec<Provider>> {
    let models = auth_providers::Entity::find()
        .all(db)
        .await
        .context("listing identity providers")?;
    models_to_providers(models)
}

/// A provider by id, deleted or not.
pub async fn find_any(db: &impl ConnectionTrait, id: &str) -> anyhow::Result<Option<Provider>> {
    auth_providers::Entity::find_by_id(id.to_owned())
        .one(db)
        .await
        .context("loading an identity provider")?
        .map(Provider::from_model)
        .transpose()
}

/// A non-deleted provider; `provider_not_found` otherwise.
pub async fn find(db: &impl ConnectionTrait, id: &str) -> Result<Provider, ApiError> {
    match find_any(db, id).await? {
        Some(p) if p.deleted_at.is_none() => Ok(p),
        _ => Err(ApiError::ProviderNotFound),
    }
}

/// One more than the highest position (0 for the first provider).
pub async fn next_position(db: &impl ConnectionTrait) -> anyhow::Result<i32> {
    let max: Option<Option<i32>> = auth_providers::Entity::find()
        .select_only()
        .column_as(
            Expr::expr(Func::max(Expr::col(auth_providers::Column::Position))),
            "max",
        )
        .into_tuple()
        .one(db)
        .await
        .context("reading provider positions")?;
    Ok(max.flatten().map_or(0, |m| m.saturating_add(1)))
}

/// Inserts a provider; `provider_id_taken` if the id exists (deleted ones included).
pub async fn insert(
    db: &impl ConnectionTrait,
    secrets: &SecretBox,
    id: &str,
    kind: ProviderKind,
    fields: &ProviderFields,
    client_secret: &str,
) -> Result<Provider, ApiError> {
    let now = crate::time::now();
    let model = auth_providers::ActiveModel {
        id: Set(id.to_owned()),
        kind: Set(kind.as_str().to_owned()),
        display_name: Set(fields.display_name.clone()),
        icon: Set(fields.icon.clone()),
        issuer_url: Set(fields.issuer_url.clone()),
        tenant: Set(fields.tenant.clone()),
        client_id: Set(fields.client_id.clone()),
        client_secret_enc: Set(secrets.encrypt(&secret_aad(id), client_secret.as_bytes())?),
        scopes: Set(fields.scopes.clone()),
        groups_claim: Set(fields.groups_claim.clone()),
        admin_groups: Set(groups_json(&fields.admin_groups)?),
        allowed_groups: Set(groups_json(&fields.allowed_groups)?),
        enabled: Set(fields.enabled),
        position: Set(fields.position),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    };
    match auth_providers::Entity::insert(model)
        .exec_with_returning(db)
        .await
    {
        Ok(model) => Ok(Provider::from_model(model)?),
        Err(err) if crate::accounts::users::is_unique_violation(&err) => {
            Err(ApiError::ProviderIdTaken)
        }
        Err(err) => Err(anyhow::Error::new(err)
            .context("inserting an identity provider")
            .into()),
    }
}

/// Overwrites the editable fields (and the secret, if given) of a non-deleted
/// provider; `provider_not_found` if it is gone.
pub async fn update(
    db: &impl ConnectionTrait,
    secrets: &SecretBox,
    id: &str,
    fields: &ProviderFields,
    client_secret: Option<&str>,
) -> Result<Provider, ApiError> {
    let mut update = auth_providers::Entity::update_many()
        .col_expr(
            auth_providers::Column::DisplayName,
            Expr::value(fields.display_name.clone()),
        )
        .col_expr(
            auth_providers::Column::Icon,
            Expr::value(fields.icon.clone()),
        )
        .col_expr(
            auth_providers::Column::IssuerUrl,
            Expr::value(fields.issuer_url.clone()),
        )
        .col_expr(
            auth_providers::Column::Tenant,
            Expr::value(fields.tenant.clone()),
        )
        .col_expr(
            auth_providers::Column::ClientId,
            Expr::value(fields.client_id.clone()),
        )
        .col_expr(
            auth_providers::Column::Scopes,
            Expr::value(fields.scopes.clone()),
        )
        .col_expr(
            auth_providers::Column::GroupsClaim,
            Expr::value(fields.groups_claim.clone()),
        )
        .col_expr(
            auth_providers::Column::AdminGroups,
            Expr::value(groups_json(&fields.admin_groups)?),
        )
        .col_expr(
            auth_providers::Column::AllowedGroups,
            Expr::value(groups_json(&fields.allowed_groups)?),
        )
        .col_expr(auth_providers::Column::Enabled, Expr::value(fields.enabled))
        .col_expr(
            auth_providers::Column::Position,
            Expr::value(fields.position),
        )
        .col_expr(
            auth_providers::Column::UpdatedAt,
            Expr::value(crate::time::now()),
        )
        .filter(auth_providers::Column::Id.eq(id))
        .filter(auth_providers::Column::DeletedAt.is_null());
    if let Some(secret) = client_secret {
        update = update.col_expr(
            auth_providers::Column::ClientSecretEnc,
            Expr::value(secrets.encrypt(&secret_aad(id), secret.as_bytes())?),
        );
    }
    let result = update
        .exec(db)
        .await
        .context("updating an identity provider")?;
    if result.rows_affected != 1 {
        return Err(ApiError::ProviderNotFound);
    }
    find(db, id).await
}

/// Soft-deletes (and disables) a provider; `false` if there is no such live provider.
pub async fn soft_delete(db: &impl ConnectionTrait, id: &str) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = auth_providers::Entity::update_many()
        .col_expr(auth_providers::Column::DeletedAt, Expr::value(now))
        .col_expr(auth_providers::Column::Enabled, Expr::value(false))
        .col_expr(auth_providers::Column::UpdatedAt, Expr::value(now))
        .filter(auth_providers::Column::Id.eq(id))
        .filter(auth_providers::Column::DeletedAt.is_null())
        .exec(db)
        .await
        .context("deleting an identity provider")?;
    Ok(result.rows_affected == 1)
}
