//! Server settings in the `settings` table: one JSON value per key, editable by admins.
//! A missing key means the default. Keys so far: `registration_mode` (default `invite`).

use anyhow::Context as _;
use cloudit_core::protocol::RegistrationMode;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ConnectionTrait, EntityTrait as _};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::db::entities::settings;

pub const REGISTRATION_MODE: &str = "registration_mode";

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
