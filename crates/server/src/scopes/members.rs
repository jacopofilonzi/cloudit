//! The `scope_members` table. Access checks are the caller's job ([`super::access`]).

use anyhow::Context as _;
use cloudit_core::protocol::{MemberRole, ScopeRole};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use crate::db::entities::scope_members;

/// The member's role; an unknown stored value counts as `viewer` (fails closed).
#[must_use]
pub fn role_of(member: &scope_members::Model) -> ScopeRole {
    MemberRole::parse(&member.role).map_or(ScopeRole::Viewer, ScopeRole::from)
}

pub async fn find(
    db: &impl ConnectionTrait,
    scope: Uuid,
    user: Uuid,
) -> anyhow::Result<Option<scope_members::Model>> {
    scope_members::Entity::find_by_id((scope, user))
        .one(db)
        .await
        .context("loading a scope member")
}

/// The members of a scope (without the owner), oldest first.
pub async fn list(
    db: &impl ConnectionTrait,
    scope: Uuid,
) -> anyhow::Result<Vec<scope_members::Model>> {
    scope_members::Entity::find()
        .filter(scope_members::Column::ScopeId.eq(scope))
        .order_by_asc(scope_members::Column::CreatedAt)
        .order_by_asc(scope_members::Column::UserId)
        .all(db)
        .await
        .context("listing scope members")
}

/// Adds `user` with `role`, or changes the role of an existing member. Returns the row
/// and whether it was created.
pub async fn upsert(
    db: &impl ConnectionTrait,
    scope: Uuid,
    user: Uuid,
    role: MemberRole,
    by: Uuid,
) -> anyhow::Result<(scope_members::Model, bool)> {
    if let Some(member) = update_role(db, scope, user, role).await? {
        return Ok((member, false));
    }
    let now = crate::time::now();
    let model = scope_members::ActiveModel {
        scope_id: Set(scope),
        user_id: Set(user),
        role: Set(role.as_str().to_owned()),
        added_by: Set(by),
        created_at: Set(now),
        updated_at: Set(now),
    };
    // A concurrent add of the same user turns into a role change.
    scope_members::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([
                scope_members::Column::ScopeId,
                scope_members::Column::UserId,
            ])
            .update_columns([
                scope_members::Column::Role,
                scope_members::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .context("adding a scope member")?;
    let member = find(db, scope, user)
        .await?
        .context("scope member vanished after insert")?;
    Ok((member, true))
}

/// Changes a member's role; `None` if `user` is not a member.
pub async fn update_role(
    db: &impl ConnectionTrait,
    scope: Uuid,
    user: Uuid,
    role: MemberRole,
) -> anyhow::Result<Option<scope_members::Model>> {
    let result = scope_members::Entity::update_many()
        .col_expr(scope_members::Column::Role, Expr::value(role.as_str()))
        .col_expr(
            scope_members::Column::UpdatedAt,
            Expr::value(crate::time::now()),
        )
        .filter(scope_members::Column::ScopeId.eq(scope))
        .filter(scope_members::Column::UserId.eq(user))
        .exec(db)
        .await
        .context("changing a member's role")?;
    if result.rows_affected == 0 {
        return Ok(None);
    }
    find(db, scope, user).await
}

/// Removes a member; `false` if `user` was not one.
pub async fn remove(db: &impl ConnectionTrait, scope: Uuid, user: Uuid) -> anyhow::Result<bool> {
    let result = scope_members::Entity::delete_many()
        .filter(scope_members::Column::ScopeId.eq(scope))
        .filter(scope_members::Column::UserId.eq(user))
        .exec(db)
        .await
        .context("removing a scope member")?;
    Ok(result.rows_affected == 1)
}
