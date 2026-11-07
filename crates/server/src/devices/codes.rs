//! Authorization codes of the device login: created by `authorize` for the signed-in
//! user, exchanged once at `token` within [`TTL`].
//!
//! The exchange consumes the code first (one conditional UPDATE on `used_at`), then
//! checks the redirect URI and the PKCE verifier: a wrong verifier burns the code, so
//! it can't be brute-forced.

use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use uuid::Uuid;

use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::device_auth_codes;

/// Validity of an authorization code.
pub const TTL: Duration = Duration::from_secs(120);

/// What `authorize` stores with the code.
pub struct NewCode {
    pub user_id: Uuid,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub device_name: String,
    pub os: String,
    pub token_ttl: Duration,
}

/// Stores a new code; returns it in plaintext.
pub async fn create(db: &impl ConnectionTrait, new: NewCode) -> anyhow::Result<Secret> {
    let code = tokens::new_token()?;
    let now = crate::time::now();
    let model = device_auth_codes::ActiveModel {
        id: Set(Uuid::now_v7()),
        code_hash: Set(tokens::hash_token(code.expose())),
        user_id: Set(new.user_id),
        code_challenge: Set(new.code_challenge),
        redirect_uri: Set(new.redirect_uri),
        device_name: Set(new.device_name),
        os: Set(new.os),
        token_ttl_secs: Set(i64::try_from(new.token_ttl.as_secs()).context("token TTL")?),
        created_at: Set(now),
        expires_at: Set(crate::time::add(now, TTL)?),
        used_at: Set(None),
    };
    device_auth_codes::Entity::insert(model)
        .exec_without_returning(db)
        .await
        .context("storing an authorization code")?;
    Ok(code)
}

/// Marks the code used and returns it, or `None` if it is unknown, used or expired.
/// Call first in the transaction that creates the device.
pub async fn consume(
    db: &impl ConnectionTrait,
    code: &str,
) -> anyhow::Result<Option<device_auth_codes::Model>> {
    let hash = tokens::hash_token(code);
    let now = crate::time::now();
    let result = device_auth_codes::Entity::update_many()
        .col_expr(device_auth_codes::Column::UsedAt, Expr::value(now))
        .filter(device_auth_codes::Column::CodeHash.eq(hash.clone()))
        .filter(device_auth_codes::Column::UsedAt.is_null())
        .filter(device_auth_codes::Column::ExpiresAt.gt(now))
        .exec(db)
        .await
        .context("consuming an authorization code")?;
    if result.rows_affected != 1 {
        return Ok(None);
    }
    device_auth_codes::Entity::find()
        .filter(device_auth_codes::Column::CodeHash.eq(hash))
        .one(db)
        .await
        .context("loading an authorization code")
}
