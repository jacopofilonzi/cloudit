//! Password-reset links: created by an admin or by the CLI, valid 24 hours, single use.
//! Consuming is one conditional UPDATE (`used_at IS NULL AND expires_at > now`). A new
//! reset invalidates the user's earlier unused ones.

use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, DatabaseConnection, EntityTrait as _,
    QueryFilter as _, QuerySelect as _, TransactionTrait as _,
};
use uuid::Uuid;

use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::password_resets;

/// Validity of a reset link.
pub const TTL: Duration = Duration::from_secs(24 * 3600);

/// Creates a reset for `user_id`; `created_by` is `None` for the CLI. The user's
/// earlier unused resets stop working (marked used) in the same transaction: only the
/// newest link is valid.
pub async fn create(
    db: &DatabaseConnection,
    user_id: Uuid,
    created_by: Option<Uuid>,
) -> anyhow::Result<(Secret, password_resets::Model)> {
    let token = tokens::new_token()?;
    let now = crate::time::now();
    let tx = db.begin().await.context("starting a transaction")?;
    password_resets::Entity::update_many()
        .col_expr(password_resets::Column::UsedAt, Expr::value(now))
        .filter(password_resets::Column::UserId.eq(user_id))
        .filter(password_resets::Column::UsedAt.is_null())
        .exec(&tx)
        .await
        .context("invalidating older password resets")?;
    let model = password_resets::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(tokens::hash_token(token.expose())),
        user_id: Set(user_id),
        created_by: Set(created_by),
        created_at: Set(now),
        expires_at: Set(crate::time::add(now, TTL)?),
        used_at: Set(None),
    };
    let reset = password_resets::Entity::insert(model)
        .exec_with_returning(&tx)
        .await
        .context("creating a password reset")?;
    tx.commit().await.context("committing a password reset")?;
    Ok((token, reset))
}

/// Consumes the reset for `token`; returns its user, or `None` if the token is unknown,
/// used or expired. Call first in the transaction that sets the password.
pub async fn consume(db: &impl ConnectionTrait, token: &str) -> anyhow::Result<Option<Uuid>> {
    let hash = tokens::hash_token(token);
    let now = crate::time::now();
    let result = password_resets::Entity::update_many()
        .col_expr(password_resets::Column::UsedAt, Expr::value(now))
        .filter(password_resets::Column::TokenHash.eq(hash.clone()))
        .filter(password_resets::Column::UsedAt.is_null())
        .filter(password_resets::Column::ExpiresAt.gt(now))
        .exec(db)
        .await
        .context("consuming a password reset")?;
    if result.rows_affected != 1 {
        return Ok(None);
    }
    password_resets::Entity::find()
        .select_only()
        .column(password_resets::Column::UserId)
        .filter(password_resets::Column::TokenHash.eq(hash))
        .into_tuple::<Uuid>()
        .one(db)
        .await
        .context("loading a password reset")
}
