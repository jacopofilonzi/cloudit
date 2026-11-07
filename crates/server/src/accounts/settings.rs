//! Server settings in the `settings` table: one JSON value per key, editable by admins.
//! A missing key means the default. Keys: `registration_mode` (default `invite`),
//! `password_new_accounts` (default `approval`),
//! `autolink_email` (default `false`), `public_url` (`null`/missing = none) and
//! `secret_key_check` (see `crypto::canary`).

use anyhow::Context as _;
use cloudit_core::protocol::{NewAccounts, RegistrationMode};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ConnectionTrait, EntityTrait as _};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::db::entities::settings;

pub const REGISTRATION_MODE: &str = "registration_mode";
pub const PASSWORD_NEW_ACCOUNTS: &str = "password_new_accounts";
pub const AUTOLINK_EMAIL: &str = "autolink_email";
pub const PUBLIC_URL: &str = "public_url";

async fn get<T: DeserializeOwned>(
    db: &impl ConnectionTrait,
    key: &str,
) -> anyhow::Result<Option<T>> {
    let Some(row) = settings::Entity::find_by_id(key.to_owned())
        .one(db)
        .await
        .with_context(|| format!("reading setting {key}"))?
    else {
        return Ok(None);
    };
    match serde_json::from_str(&row.value) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            // A value this version can't read (e.g. written by a newer one): fall back
            // to the default rather than failing every request.
            tracing::warn!(key, error = %err, "unreadable setting; using the default");
            Ok(None)
        }
    }
}

async fn set<T: Serialize>(
    db: &impl ConnectionTrait,
    key: &str,
    value: &T,
    by: Option<Uuid>,
) -> anyhow::Result<()> {
    let model = settings::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(serde_json::to_string(value).context("encoding a setting")?),
        updated_at: Set(crate::time::now()),
        updated_by: Set(by),
    };
    settings::Entity::insert(model)
        .on_conflict(
            OnConflict::column(settings::Column::Key)
                .update_columns([
                    settings::Column::Value,
                    settings::Column::UpdatedAt,
                    settings::Column::UpdatedBy,
                ])
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .with_context(|| format!("writing setting {key}"))?;
    Ok(())
}

/// Reads `key`; unlike the typed getters an unreadable value is an error.
pub async fn get_strict<T: DeserializeOwned>(
    db: &impl ConnectionTrait,
    key: &str,
) -> anyhow::Result<Option<T>> {
    let Some(row) = settings::Entity::find_by_id(key.to_owned())
        .one(db)
        .await
        .with_context(|| format!("reading setting {key}"))?
    else {
        return Ok(None);
    };
    serde_json::from_str(&row.value)
        .map(Some)
        .with_context(|| format!("decoding setting {key}"))
}

/// Writes `key` only if it has no value yet (the first concurrent writer wins).
pub async fn insert_if_absent<T: Serialize>(
    db: &impl ConnectionTrait,
    key: &str,
    value: &T,
) -> anyhow::Result<()> {
    let model = settings::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(serde_json::to_string(value).context("encoding a setting")?),
        updated_at: Set(crate::time::now()),
        updated_by: Set(None),
    };
    settings::Entity::insert(model)
        .on_conflict(
            OnConflict::column(settings::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .with_context(|| format!("writing setting {key}"))?;
    Ok(())
}

pub async fn registration_mode(db: &impl ConnectionTrait) -> anyhow::Result<RegistrationMode> {
    Ok(get(db, REGISTRATION_MODE).await?.unwrap_or_default())
}

pub async fn set_registration_mode(
    db: &impl ConnectionTrait,
    mode: RegistrationMode,
    by: Option<Uuid>,
) -> anyhow::Result<()> {
    set(db, REGISTRATION_MODE, &mode, by).await
}

pub async fn autolink_email(db: &impl ConnectionTrait) -> anyhow::Result<bool> {
    Ok(get(db, AUTOLINK_EMAIL).await?.unwrap_or(false))
}

pub async fn set_autolink_email(
    db: &impl ConnectionTrait,
    on: bool,
    by: Option<Uuid>,
) -> anyhow::Result<()> {
    set(db, AUTOLINK_EMAIL, &on, by).await
}

/// The stored public URL (validated `http(s)://host[:port]`), if any. The effective
/// one is [`crate::state::AppState::public_url`] (`PUBLIC_URL` wins).
pub async fn public_url(db: &impl ConnectionTrait) -> anyhow::Result<Option<String>> {
    Ok(get::<Option<String>>(db, PUBLIC_URL).await?.flatten())
}

/// Stores (or, with `None`, clears) the public URL; validate it first.
pub async fn set_public_url(
    db: &impl ConnectionTrait,
    url: Option<&str>,
    by: Option<Uuid>,
) -> anyhow::Result<()> {
    set(db, PUBLIC_URL, &url, by).await
}

/// Whether password registrations without an invite need an admin's approval.
pub async fn password_new_accounts(db: &impl ConnectionTrait) -> anyhow::Result<NewAccounts> {
    Ok(get(db, PASSWORD_NEW_ACCOUNTS).await?.unwrap_or_default())
}

pub async fn set_password_new_accounts(
    db: &impl ConnectionTrait,
    policy: NewAccounts,
    by: Option<Uuid>,
) -> anyhow::Result<()> {
    set(db, PASSWORD_NEW_ACCOUNTS, &policy, by).await
}
