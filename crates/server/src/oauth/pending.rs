//! `pending_signups`: an authenticated identity no account owns yet, waiting for the
//! onboarding choice (create an account, or attach it to an existing one). The token
//! lives in the `cloudit_onboarding` cookie; only its SHA-256 is stored. Consuming is one
//! conditional UPDATE (`used_at IS NULL AND expires_at > now`).

use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use uuid::Uuid;

use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::pending_signups;
use crate::idp::claims::ExternalIdentity;

/// Validity of a pending signup.
pub const TTL: Duration = Duration::from_secs(15 * 60);

/// Stores a pending signup for `identity`; returns its token.
pub async fn create(
    db: &impl ConnectionTrait,
    provider_id: &str,
    identity: &ExternalIdentity,
    invite_token_hash: Option<String>,
) -> anyhow::Result<Secret> {
    let token = tokens::new_token()?;
    let now = crate::time::now();
    let model = pending_signups::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(tokens::hash_token(token.expose())),
        provider_id: Set(provider_id.to_owned()),
        subject: Set(identity.subject.clone()),
        email: Set(identity.email.clone()),
        email_verified: Set(identity.email_verified),
        display_name: Set(identity.display_name.clone()),
        groups: Set(
            serde_json::to_string(identity.groups.as_deref().unwrap_or_default())
                .context("encoding groups")?,
        ),
        invite_token_hash: Set(invite_token_hash),
        created_at: Set(now),
        expires_at: Set(crate::time::add(now, TTL)?),
        used_at: Set(None),
    };
    pending_signups::Entity::insert(model)
        .exec_without_returning(db)
        .await
        .context("storing a pending signup")?;
    Ok(token)
}

fn usable(row: Option<pending_signups::Model>) -> Option<pending_signups::Model> {
    let now = crate::time::now();
    row.filter(|p| p.used_at.is_none() && p.expires_at > now)
}

/// The unused, unexpired pending signup of `token`.
pub async fn find_usable(
    db: &impl ConnectionTrait,
    token: &str,
) -> anyhow::Result<Option<pending_signups::Model>> {
    let row = pending_signups::Entity::find()
        .filter(pending_signups::Column::TokenHash.eq(tokens::hash_token(token)))
        .one(db)
        .await
        .context("loading a pending signup")?;
    Ok(usable(row))
}

/// The unused, unexpired pending signup `id`.
pub async fn find_usable_by_id(
    db: &impl ConnectionTrait,
    id: Uuid,
) -> anyhow::Result<Option<pending_signups::Model>> {
    let row = pending_signups::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading a pending signup")?;
    Ok(usable(row))
}

/// Marks it used; `false` if it was used or expired meanwhile. Call first in the
/// transaction that creates the account or the identity.
pub async fn consume(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = pending_signups::Entity::update_many()
        .col_expr(pending_signups::Column::UsedAt, Expr::value(now))
        .filter(pending_signups::Column::Id.eq(id))
        .filter(pending_signups::Column::UsedAt.is_null())
        .filter(pending_signups::Column::ExpiresAt.gt(now))
        .exec(db)
        .await
        .context("consuming a pending signup")?;
    Ok(result.rows_affected == 1)
}

/// The identity a pending signup holds.
pub fn identity(pending: &pending_signups::Model) -> anyhow::Result<ExternalIdentity> {
    let groups: Vec<String> =
        serde_json::from_str(&pending.groups).context("decoding pending signup groups")?;
    Ok(ExternalIdentity {
        subject: pending.subject.clone(),
        email: pending.email.clone(),
        email_verified: pending.email_verified,
        display_name: pending.display_name.clone(),
        groups: Some(groups),
    })
}
