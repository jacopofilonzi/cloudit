//! The `scopes` table. Access checks are the caller's job ([`super::access`]).

use std::collections::HashMap;

use anyhow::Context as _;
use cloudit_core::protocol::ScopeRole;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QuerySelect as _,
};
use uuid::Uuid;

use super::names::Name;
use crate::accounts::users::is_unique_violation;
use crate::db::entities::{folders, scope_members, scopes};
use crate::errors::ApiError;

/// Creates a scope owned by `owner`; `name_taken` if another of its scopes outside the
/// trash has the name.
pub async fn create(
    db: &impl ConnectionTrait,
    owner: Uuid,
    name: Name,
) -> Result<scopes::Model, ApiError> {
    let now = crate::time::now();
    let model = scopes::ActiveModel {
        id: Set(Uuid::now_v7()),
        owner_id: Set(owner),
        name: Set(name.name),
        active_key: Set(Some(name.key.clone())),
        name_key: Set(name.key),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
        deleted_by: Set(None),
    };
    match scopes::Entity::insert(model).exec_with_returning(db).await {
        Ok(scope) => Ok(scope),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("creating a scope").into()),
    }
}

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<Option<scopes::Model>> {
    scopes::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading a scope")
}

/// Every scope outside the trash that `user` owns or is a member of, with the role.
pub async fn list_visible(
    db: &impl ConnectionTrait,
    user: Uuid,
) -> anyhow::Result<Vec<(scopes::Model, ScopeRole)>> {
    let owned = scopes::Entity::find()
        .filter(scopes::Column::OwnerId.eq(user))
        .filter(scopes::Column::DeletedAt.is_null())
        .all(db)
        .await
        .context("listing owned scopes")?;
    let memberships: HashMap<Uuid, ScopeRole> = scope_members::Entity::find()
        .filter(scope_members::Column::UserId.eq(user))
        .all(db)
        .await
        .context("listing memberships")?
        .into_iter()
        .map(|m| (m.scope_id, super::members::role_of(&m)))
        .collect();
    let shared = if memberships.is_empty() {
        Vec::new()
    } else {
        scopes::Entity::find()
            .filter(scopes::Column::Id.is_in(memberships.keys().copied()))
            .filter(scopes::Column::DeletedAt.is_null())
            .all(db)
            .await
            .context("listing shared scopes")?
    };
    let mut all: Vec<_> = owned
        .into_iter()
        .map(|s| (s, ScopeRole::Owner))
        .chain(shared.into_iter().filter_map(|s| {
            let role = *memberships.get(&s.id)?;
            Some((s, role))
        }))
        .collect();
    all.sort_by(|(a, _), (b, _)| a.name_key.cmp(&b.name_key).then(a.id.cmp(&b.id)));
    Ok(all)
}

/// Number of folders outside the trash per scope (scopes without any are absent).
pub async fn folder_counts(
    db: &impl ConnectionTrait,
    scope_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, u64>> {
    if scope_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, i64)> = folders::Entity::find()
        .select_only()
        .column(folders::Column::ScopeId)
        .column_as(folders::Column::Id.count(), "count")
        .filter(folders::Column::ScopeId.is_in(scope_ids.iter().copied()))
        .filter(folders::Column::DeletedAt.is_null())
        .group_by(folders::Column::ScopeId)
        .into_tuple()
        .all(db)
        .await
        .context("counting folders")?;
    Ok(rows
        .into_iter()
        .map(|(id, n)| (id, u64::try_from(n).unwrap_or(0)))
        .collect())
}

/// Renames a scope outside the trash; `name_taken` on a duplicate.
pub async fn rename(db: &impl ConnectionTrait, id: Uuid, name: Name) -> Result<(), ApiError> {
    let result = scopes::Entity::update_many()
        .col_expr(scopes::Column::Name, Expr::value(name.name))
        .col_expr(scopes::Column::NameKey, Expr::value(name.key.clone()))
        .col_expr(scopes::Column::ActiveKey, Expr::value(name.key))
        .col_expr(scopes::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(scopes::Column::Id.eq(id))
        .filter(scopes::Column::DeletedAt.is_null())
        .exec(db)
        .await;
    match result {
        Ok(r) if r.rows_affected == 1 => Ok(()),
        Ok(_) => Err(ApiError::NotFound),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("renaming a scope").into()),
    }
}

/// Moves a scope to the trash; `false` if it is already there (or gone).
pub async fn trash(db: &impl ConnectionTrait, id: Uuid, by: Uuid) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = scopes::Entity::update_many()
        .col_expr(scopes::Column::DeletedAt, Expr::value(now))
        .col_expr(scopes::Column::DeletedBy, Expr::value(by))
        .col_expr(scopes::Column::ActiveKey, Expr::value(None::<String>))
        .col_expr(scopes::Column::UpdatedAt, Expr::value(now))
        .filter(scopes::Column::Id.eq(id))
        .filter(scopes::Column::DeletedAt.is_null())
        .exec(db)
        .await
        .context("moving a scope to the trash")?;
    Ok(result.rows_affected == 1)
}

/// Restores a scope from the trash, under `name` (its old one if `None`);
/// `name_taken` if the name is in use meanwhile, `not_found` if it isn't in the trash.
pub async fn restore(
    db: &impl ConnectionTrait,
    scope: &scopes::Model,
    name: Option<Name>,
) -> Result<(), ApiError> {
    let name = name.unwrap_or_else(|| Name {
        name: scope.name.clone(),
        key: scope.name_key.clone(),
    });
    let result = scopes::Entity::update_many()
        .col_expr(scopes::Column::Name, Expr::value(name.name))
        .col_expr(scopes::Column::NameKey, Expr::value(name.key.clone()))
        .col_expr(scopes::Column::ActiveKey, Expr::value(name.key))
        .col_expr(scopes::Column::DeletedAt, super::folders::null_timestamp())
        .col_expr(scopes::Column::DeletedBy, Expr::value(None::<Uuid>))
        .col_expr(scopes::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(scopes::Column::Id.eq(scope.id))
        .filter(scopes::Column::DeletedAt.is_not_null())
        .exec(db)
        .await;
    match result {
        Ok(r) if r.rows_affected == 1 => Ok(()),
        Ok(_) => Err(ApiError::NotFound),
        Err(err) if is_unique_violation(&err) => Err(ApiError::NameTaken),
        Err(err) => Err(anyhow::Error::new(err).context("restoring a scope").into()),
    }
}

/// The scopes `user` owns that are in the trash.
pub async fn trashed_owned(
    db: &impl ConnectionTrait,
    user: Uuid,
) -> anyhow::Result<Vec<scopes::Model>> {
    scopes::Entity::find()
        .filter(scopes::Column::OwnerId.eq(user))
        .filter(scopes::Column::DeletedAt.is_not_null())
        .all(db)
        .await
        .context("listing trashed scopes")
}

/// Scopes outside the trash where `user` is owner or admin (whose trashed folders the
/// user may restore).
pub async fn managed(db: &impl ConnectionTrait, user: Uuid) -> anyhow::Result<Vec<scopes::Model>> {
    Ok(list_visible(db, user)
        .await?
        .into_iter()
        .filter(|(_, role)| *role >= ScopeRole::Admin)
        .map(|(scope, _)| scope)
        .collect())
}
