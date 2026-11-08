//! The `folders` table. Access checks are the caller's job ([`super::access`]).

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use super::names::Name;
use crate::accounts::users::is_unique_violation;
use crate::db::entities::folders;
use crate::errors::ApiError;

/// Per-folder retention overrides, already validated (`None` = server default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Retention {
    pub max_versions: Option<i32>,
    pub max_per_day: Option<i32>,
}

/// Creates a folder in `scope`; `name_taken` on a duplicate.
pub async fn create(
    db: &impl ConnectionTrait,
    scope: Uuid,
    name: Name,
    retention: Retention,
    by: Uuid,
) -> Result<folders::Model, ApiError> {
    let now = crate::time::now();
    let model = folders::ActiveModel {
        id: Set(Uuid::now_v7()),
        scope_id: Set(scope),
        name: Set(name.name),
        active_key: Set(Some(name.key.clone())),
        name_key: Set(name.key),
        head_snapshot_id: Set(None),
        retention_max_versions: Set(retention.max_versions),
        retention_max_per_day: Set(retention.max_per_day),
        created_by: Set(by),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
        deleted_by: Set(None),
        purge_requested_at: Set(None),
    };
    match folders::Entity::insert(model).exec_with_returning(db).await {
        Ok(folder) => Ok(folder),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("creating a folder").into()),
    }
}

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<Option<folders::Model>> {
    folders::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading a folder")
}

/// The folders of `scope` outside the trash, by name.
pub async fn list(db: &impl ConnectionTrait, scope: Uuid) -> anyhow::Result<Vec<folders::Model>> {
    folders::Entity::find()
        .filter(folders::Column::ScopeId.eq(scope))
        .filter(folders::Column::DeletedAt.is_null())
        .order_by_asc(folders::Column::NameKey)
        .order_by_asc(folders::Column::Id)
        .all(db)
        .await
        .context("listing folders")
}

/// Changes to a folder; `None` fields stay unchanged.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub name: Option<Name>,
    /// `Some(None)` resets to the server default.
    pub max_versions: Option<Option<i32>>,
    pub max_per_day: Option<Option<i32>>,
}

/// Applies `changes` to a folder outside the trash; `name_taken` on a duplicate name.
pub async fn update(db: &impl ConnectionTrait, id: Uuid, changes: Changes) -> Result<(), ApiError> {
    let mut update = folders::Entity::update_many()
        .col_expr(folders::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(folders::Column::Id.eq(id))
        .filter(folders::Column::DeletedAt.is_null());
    if let Some(name) = changes.name {
        update = update
            .col_expr(folders::Column::Name, Expr::value(name.name))
            .col_expr(folders::Column::NameKey, Expr::value(name.key.clone()))
            .col_expr(folders::Column::ActiveKey, Expr::value(name.key));
    }
    if let Some(value) = changes.max_versions {
        update = update.col_expr(folders::Column::RetentionMaxVersions, Expr::value(value));
    }
    if let Some(value) = changes.max_per_day {
        update = update.col_expr(folders::Column::RetentionMaxPerDay, Expr::value(value));
    }
    match update.exec(db).await {
        Ok(r) if r.rows_affected == 1 => Ok(()),
        Ok(_) => Err(ApiError::NotFound),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("updating a folder").into()),
    }
}

/// Moves a folder to the trash; `false` if it is already there (or gone).
pub async fn trash(db: &impl ConnectionTrait, id: Uuid, by: Uuid) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = folders::Entity::update_many()
        .col_expr(folders::Column::DeletedAt, Expr::value(now))
        .col_expr(folders::Column::DeletedBy, Expr::value(by))
        .col_expr(folders::Column::ActiveKey, Expr::value(None::<String>))
        .col_expr(folders::Column::UpdatedAt, Expr::value(now))
        .filter(folders::Column::Id.eq(id))
        .filter(folders::Column::DeletedAt.is_null())
        .exec(db)
        .await
        .context("moving a folder to the trash")?;
    Ok(result.rows_affected == 1)
}

/// Restores a folder from the trash under `name` (its old one if `None`);
/// `name_taken` if the name is in use meanwhile, `not_found` if it isn't in the trash.
pub async fn restore(
    db: &impl ConnectionTrait,
    folder: &folders::Model,
    name: Option<Name>,
) -> Result<(), ApiError> {
    let name = name.unwrap_or_else(|| Name {
        name: folder.name.clone(),
        key: folder.name_key.clone(),
    });
    let result = folders::Entity::update_many()
        .col_expr(folders::Column::Name, Expr::value(name.name))
        .col_expr(folders::Column::NameKey, Expr::value(name.key.clone()))
        .col_expr(folders::Column::ActiveKey, Expr::value(name.key))
        .col_expr(folders::Column::DeletedAt, null_timestamp())
        .col_expr(folders::Column::DeletedBy, Expr::value(None::<Uuid>))
        .col_expr(folders::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(folders::Column::Id.eq(folder.id))
        .filter(folders::Column::DeletedAt.is_not_null())
        .filter(folders::Column::PurgeRequestedAt.is_null())
        .exec(db)
        .await;
    match result {
        Ok(r) if r.rows_affected == 1 => Ok(()),
        Ok(_) => Err(ApiError::NotFound),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("restoring a folder").into()),
    }
}

/// The trashed folders of these scopes.
pub async fn trashed_in(
    db: &impl ConnectionTrait,
    scopes: &[Uuid],
) -> anyhow::Result<Vec<folders::Model>> {
    if scopes.is_empty() {
        return Ok(Vec::new());
    }
    folders::Entity::find()
        .filter(folders::Column::ScopeId.is_in(scopes.iter().copied()))
        .filter(folders::Column::DeletedAt.is_not_null())
        .filter(folders::Column::PurgeRequestedAt.is_null())
        .all(db)
        .await
        .context("listing trashed folders")
}

/// `NULL` for a timestamp column.
pub fn null_timestamp() -> SimpleExpr {
    Expr::value(None::<DateTime<Utc>>)
}

/// Marks a trashed folder for deletion for good (the `trash_purge` job deletes it);
/// `false` if it isn't in the trash or is already marked.
pub async fn request_purge(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<bool> {
    let result = folders::Entity::update_many()
        .col_expr(
            folders::Column::PurgeRequestedAt,
            Expr::value(crate::time::now()),
        )
        .filter(folders::Column::Id.eq(id))
        .filter(folders::Column::DeletedAt.is_not_null())
        .filter(folders::Column::PurgeRequestedAt.is_null())
        .exec(db)
        .await
        .context("marking a folder for purge")?;
    Ok(result.rows_affected == 1)
}
