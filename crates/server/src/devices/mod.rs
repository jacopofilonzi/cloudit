//! Desktop devices: login through loopback redirect + PKCE ([`codes`], [`redirect`])
//! and the `devices` table (bearer tokens `cld_…`).
//!
//! A device token expires at a fixed time chosen at login (not extended by use; `forever`
//! = no expiry, `expires_at` NULL) and can be revoked (soft: `revoked_at`). `last_seen_at` is written at most every
//! [`SEEN_EVERY`].

pub mod codes;
pub mod redirect;

use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use crate::accounts::users;
use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::{devices, users as user_entity};

/// Minimum interval between two writes of `last_seen_at`.
pub const SEEN_EVERY: Duration = Duration::from_secs(5 * 60);

/// Creates a device with a new token valid until `expires_at` (`None` = until revoked);
/// returns the token.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    name: String,
    os: String,
    expires_at: Option<DateTime<Utc>>,
) -> anyhow::Result<(Secret, devices::Model)> {
    let token = tokens::new_device_token()?;
    let model = devices::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        name: Set(name),
        os: Set(os),
        token_hash: Set(tokens::hash_token(token.expose())),
        created_at: Set(crate::time::now()),
        last_seen_at: Set(None),
        expires_at: Set(expires_at),
        revoked_at: Set(None),
    };
    let device = devices::Entity::insert(model)
        .exec_with_returning(db)
        .await
        .context("creating a device")?;
    Ok((token, device))
}

/// The unrevoked, unexpired device for a bearer `token`, and its user.
pub async fn authenticate(
    db: &impl ConnectionTrait,
    token: &str,
) -> anyhow::Result<Option<(devices::Model, user_entity::Model)>> {
    if !token.starts_with(tokens::DEVICE_TOKEN_PREFIX) {
        return Ok(None);
    }
    let device = devices::Entity::find()
        .filter(devices::Column::TokenHash.eq(tokens::hash_token(token)))
        .filter(devices::Column::RevokedAt.is_null())
        .filter(
            devices::Column::ExpiresAt
                .is_null()
                .or(devices::Column::ExpiresAt.gt(crate::time::now())),
        )
        .one(db)
        .await
        .context("loading a device")?;
    let Some(device) = device else {
        return Ok(None);
    };
    Ok(users::find(db, device.user_id)
        .await?
        .map(|user| (device, user)))
}

/// Records a use of the device if the last recorded one is older than [`SEEN_EVERY`].
pub async fn touch(
    db: &impl ConnectionTrait,
    device: &devices::Model,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let due_before = now - chrono::TimeDelta::from_std(SEEN_EVERY).context("seen interval")?;
    if device.last_seen_at.is_some_and(|seen| seen > due_before) {
        return Ok(());
    }
    devices::Entity::update_many()
        .col_expr(devices::Column::LastSeenAt, Expr::value(now))
        .filter(devices::Column::Id.eq(device.id))
        .filter(
            devices::Column::LastSeenAt
                .is_null()
                .or(devices::Column::LastSeenAt.lte(due_before)),
        )
        .exec(db)
        .await
        .context("recording device use")?;
    Ok(())
}

/// The user's devices, newest first (revoked and expired included).
pub async fn list(db: &impl ConnectionTrait, user_id: Uuid) -> anyhow::Result<Vec<devices::Model>> {
    devices::Entity::find()
        .filter(devices::Column::UserId.eq(user_id))
        .order_by_desc(devices::Column::CreatedAt)
        .all(db)
        .await
        .context("listing devices")
}

/// One of the user's devices.
pub async fn find(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    id: Uuid,
) -> anyhow::Result<Option<devices::Model>> {
    devices::Entity::find_by_id(id)
        .filter(devices::Column::UserId.eq(user_id))
        .one(db)
        .await
        .context("loading a device")
}

/// Renames one of the user's devices; `false` if there is no such device.
pub async fn rename(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    id: Uuid,
    name: String,
) -> anyhow::Result<bool> {
    let result = devices::Entity::update_many()
        .col_expr(devices::Column::Name, Expr::value(name))
        .filter(devices::Column::Id.eq(id))
        .filter(devices::Column::UserId.eq(user_id))
        .exec(db)
        .await
        .context("renaming a device")?;
    Ok(result.rows_affected == 1)
}

/// Revokes one of the user's devices (soft); `false` if there is no such device.
/// Revoking twice keeps the first time.
pub async fn revoke(db: &impl ConnectionTrait, user_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    if find(db, user_id, id).await?.is_none() {
        return Ok(false);
    }
    devices::Entity::update_many()
        .col_expr(devices::Column::RevokedAt, Expr::value(crate::time::now()))
        .filter(devices::Column::Id.eq(id))
        .filter(devices::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context("revoking a device")?;
    Ok(true)
}
