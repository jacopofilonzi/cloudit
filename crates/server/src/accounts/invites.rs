//! Registration invites. The token is returned once at creation; only its hash is
//! stored. Consuming is one conditional UPDATE (`used_at IS NULL AND revoked_at IS NULL
//! AND expires_at > now`), so an invite registers exactly one account.

use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloudit_core::protocol::InviteStatus;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::invites;

/// Creates an invite; returns the plaintext token and the row.
pub async fn create(
    db: &impl ConnectionTrait,
    created_by: Uuid,
    email: Option<String>,
    ttl: Duration,
) -> anyhow::Result<(Secret, invites::Model)> {
    let token = tokens::new_token()?;
    let now = crate::time::now();
    let model = invites::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(tokens::hash_token(token.expose())),
        created_by: Set(created_by),
        email: Set(email),
        created_at: Set(now),
        expires_at: Set(crate::time::add(now, ttl)?),
        used_at: Set(None),
        used_by: Set(None),
        revoked_at: Set(None),
    };
    let invite = invites::Entity::insert(model)
        .exec_with_returning(db)
        .await
        .context("creating an invite")?;
    Ok((token, invite))
}

/// The invite for `token` if it is still usable (not used, revoked or expired).
pub async fn find_usable(
    db: &impl ConnectionTrait,
    token: &str,
) -> anyhow::Result<Option<invites::Model>> {
    find_usable_by_hash(db, &tokens::hash_token(token)).await
}

/// [`find_usable`] by the token's stored hash (an invite carried through a provider
/// sign-in keeps only the hash).
pub async fn find_usable_by_hash(
    db: &impl ConnectionTrait,
    token_hash: &str,
) -> anyhow::Result<Option<invites::Model>> {
    let invite = invites::Entity::find()
        .filter(invites::Column::TokenHash.eq(token_hash))
        .one(db)
        .await
        .context("loading an invite")?;
    Ok(invite.filter(|i| status(i, crate::time::now()) == InviteStatus::Pending))
}

/// Marks the invite used by `user_id`; `false` if someone else consumed it first or it
/// is no longer usable.
pub async fn consume(db: &impl ConnectionTrait, id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = invites::Entity::update_many()
        .col_expr(invites::Column::UsedAt, Expr::value(now))
        .col_expr(invites::Column::UsedBy, Expr::value(user_id))
        .filter(invites::Column::Id.eq(id))
        .filter(invites::Column::UsedAt.is_null())
        .filter(invites::Column::RevokedAt.is_null())
        .filter(invites::Column::ExpiresAt.gt(now))
        .exec(db)
        .await
        .context("consuming an invite")?;
    Ok(result.rows_affected == 1)
}

/// Every invite, newest first.
pub async fn list(db: &impl ConnectionTrait) -> anyhow::Result<Vec<invites::Model>> {
    invites::Entity::find()
        .order_by_desc(invites::Column::CreatedAt)
        .all(db)
        .await
        .context("listing invites")
}

/// Revokes an unused invite (soft). `None` if there is no such invite; revoking a used
/// or already revoked one changes nothing.
pub async fn revoke(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<Option<()>> {
    if invites::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading an invite")?
        .is_none()
    {
        return Ok(None);
    }
    invites::Entity::update_many()
        .col_expr(invites::Column::RevokedAt, Expr::value(crate::time::now()))
        .filter(invites::Column::Id.eq(id))
        .filter(invites::Column::UsedAt.is_null())
        .filter(invites::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context("revoking an invite")?;
    Ok(Some(()))
}

/// The invite's state at `now`.
#[must_use]
pub fn status(invite: &invites::Model, now: DateTime<Utc>) -> InviteStatus {
    if invite.used_at.is_some() {
        InviteStatus::Used
    } else if invite.revoked_at.is_some() {
        InviteStatus::Revoked
    } else if invite.expires_at <= now {
        InviteStatus::Expired
    } else {
        InviteStatus::Pending
    }
}
