//! Web sessions: the `cloudit_session` cookie and the `sessions` table.
//!
//! - Cookie: `HttpOnly; SameSite=Lax; Path=/`, plus `Secure` when the server is
//!   reached over https ([`super::client::ClientInfo::https`]); `Max-Age` = [`TTL`].
//! - Sliding expiry: a session expires [`TTL`] after its last use, but `last_seen_at` /
//!   `expires_at` are written at most once per [`SLIDE_EVERY`] (one write per hour per
//!   active session, not one per request); the cookie's `Max-Age` is refreshed then too.
//!   A session therefore lives between `TTL - SLIDE_EVERY` and `TTL` after its last use.
//! - Only the SHA-256 of the token is stored; logout and revocation delete the row.

use std::time::Duration;

use anyhow::Context as _;
use axum::http::{HeaderMap, HeaderValue, header};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use super::tokens;
use crate::accounts::users;
use crate::config::Secret;
use crate::db::entities::{sessions, users as user_entity};

pub const COOKIE_NAME: &str = "cloudit_session";
/// Session lifetime after the last (recorded) use.
pub const TTL: Duration = Duration::from_secs(30 * 86_400);
/// Minimum interval between two writes of `last_seen_at` / `expires_at`.
pub const SLIDE_EVERY: Duration = Duration::from_secs(3600);
/// Longest `user_agent` kept, in characters.
const MAX_USER_AGENT_CHARS: usize = 256;

/// Creates a session for `user_id`; returns the cookie token and the row.
pub async fn create(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    user_agent: Option<&str>,
) -> anyhow::Result<(Secret, sessions::Model)> {
    let token = tokens::new_token()?;
    let now = crate::time::now();
    let model = sessions::ActiveModel {
        id: Set(Uuid::now_v7()),
        token_hash: Set(tokens::hash_token(token.expose())),
        user_id: Set(user_id),
        created_at: Set(now),
        last_seen_at: Set(now),
        expires_at: Set(crate::time::add(now, TTL)?),
        user_agent: Set(user_agent.map(|ua| ua.chars().take(MAX_USER_AGENT_CHARS).collect())),
    };
    let session = sessions::Entity::insert(model)
        .exec_with_returning(db)
        .await
        .context("creating a session")?;
    Ok((token, session))
}

/// The unexpired session for `token` and its user.
pub async fn authenticate(
    db: &impl ConnectionTrait,
    token: &str,
) -> anyhow::Result<Option<(sessions::Model, user_entity::Model)>> {
    let session = sessions::Entity::find()
        .filter(sessions::Column::TokenHash.eq(tokens::hash_token(token)))
        .filter(sessions::Column::ExpiresAt.gt(crate::time::now()))
        .one(db)
        .await
        .context("loading a session")?;
    let Some(session) = session else {
        return Ok(None);
    };
    Ok(users::find(db, session.user_id)
        .await?
        .map(|user| (session, user)))
}

/// Extends the session if its last recorded use is older than [`SLIDE_EVERY`]. Returns
/// whether it was extended (then the cookie should be refreshed).
pub async fn slide_if_due(
    db: &impl ConnectionTrait,
    session: &sessions::Model,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let due_before = now - chrono::TimeDelta::from_std(SLIDE_EVERY).context("slide interval")?;
    if session.last_seen_at > due_before {
        return Ok(false);
    }
    // Conditional, so concurrent requests of one session write once.
    let result = sessions::Entity::update_many()
        .col_expr(sessions::Column::LastSeenAt, Expr::value(now))
        .col_expr(
            sessions::Column::ExpiresAt,
            Expr::value(crate::time::add(now, TTL)?),
        )
        .filter(sessions::Column::Id.eq(session.id))
        .filter(sessions::Column::LastSeenAt.lte(due_before))
        .exec(db)
        .await
        .context("extending a session")?;
    Ok(result.rows_affected == 1)
}

/// The user's unexpired sessions, most recently used first.
pub async fn list(
    db: &impl ConnectionTrait,
    user_id: Uuid,
) -> anyhow::Result<Vec<sessions::Model>> {
    sessions::Entity::find()
        .filter(sessions::Column::UserId.eq(user_id))
        .filter(sessions::Column::ExpiresAt.gt(crate::time::now()))
        .order_by_desc(sessions::Column::LastSeenAt)
        .all(db)
        .await
        .context("listing sessions")
}

/// Deletes one of the user's sessions; `false` if there is no such session.
pub async fn delete(db: &impl ConnectionTrait, user_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    let result = sessions::Entity::delete_many()
        .filter(sessions::Column::Id.eq(id))
        .filter(sessions::Column::UserId.eq(user_id))
        .exec(db)
        .await
        .context("deleting a session")?;
    Ok(result.rows_affected == 1)
}

/// Deletes all the user's sessions except `keep`.
pub async fn delete_all(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    keep: Option<Uuid>,
) -> anyhow::Result<()> {
    let mut delete = sessions::Entity::delete_many().filter(sessions::Column::UserId.eq(user_id));
    if let Some(keep) = keep {
        delete = delete.filter(sessions::Column::Id.ne(keep));
    }
    delete.exec(db).await.context("deleting sessions")?;
    Ok(())
}

fn cookie(value: &str, max_age: u64, secure: bool) -> anyhow::Result<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .context("building the session cookie")
}

/// `Set-Cookie` value carrying `token`.
pub fn set_cookie(token: &Secret, secure: bool) -> anyhow::Result<HeaderValue> {
    cookie(token.expose(), TTL.as_secs(), secure)
}

/// `Set-Cookie` value deleting the cookie.
pub fn clear_cookie(secure: bool) -> anyhow::Result<HeaderValue> {
    cookie("", 0, secure)
}

/// The session token from the `Cookie` header(s), if any.
#[must_use]
pub fn read_cookie(headers: &HeaderMap) -> Option<Secret> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == COOKIE_NAME && !value.is_empty())
        .map(|(_, value)| Secret::new(value))
}

/// `true` if `headers` already set (or cleared) the session cookie.
#[must_use]
pub fn sets_cookie(headers: &HeaderMap) -> bool {
    let prefix = format!("{COOKIE_NAME}=");
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|v| v.as_bytes().starts_with(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_attributes() {
        let c = set_cookie(&Secret::new("abc"), true).unwrap();
        assert_eq!(
            c,
            "cloudit_session=abc; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000; Secure"
        );
        let c = clear_cookie(false).unwrap();
        assert_eq!(
            c,
            "cloudit_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
        );
    }

    #[test]
    fn reads_the_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; cloudit_session=tok; other=1"),
        );
        assert_eq!(read_cookie(&headers).unwrap().expose(), "tok");
        headers.insert(header::COOKIE, HeaderValue::from_static("cloudit_session="));
        assert!(read_cookie(&headers).is_none());
    }
}
