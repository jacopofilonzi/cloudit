//! Admin changes to an account (`admin` flag, disable) with the last-admin guard.
//!
//! The guard must hold under concurrency: two admins demoting each other at the same
//! time must not both succeed. A plain "count the other admins, then update" races
//! (each transaction still sees the other admin), so every such change first takes
//! the same write lock, [`lock_admin_changes`], which serializes them: on PostgreSQL a
//! row lock on `setup_state` (id 1), on SQLite the database write lock (the first
//! statement of the transaction is a write). After the lock the check sees every
//! committed change.
//!
//! Disabling an account deletes all its web sessions in the same transaction, so
//! re-enabling it does not sign anyone back in; device tokens are kept (refused while
//! disabled).

use anyhow::Context as _;
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ColumnTrait as _, ConnectionTrait, DatabaseConnection, DeriveIden, EntityTrait as _,
    PaginatorTrait as _, QueryFilter as _, TransactionTrait as _,
};
use uuid::Uuid;

use super::users;
use crate::db::entities::users as user_entity;
use crate::errors::ApiError;

#[derive(DeriveIden)]
enum SetupState {
    Table,
    Id,
    CreatedAt,
}

/// Serializes admin-flag changes (see the module docs). Call first in the transaction.
async fn lock_admin_changes(db: &impl ConnectionTrait) -> anyhow::Result<()> {
    let stmt = Query::update()
        .table(SetupState::Table)
        .value(SetupState::CreatedAt, Expr::col(SetupState::CreatedAt))
        .and_where(Expr::col(SetupState::Id).eq(1))
        .to_owned();
    db.execute(&stmt).await.context("locking admin changes")?;
    Ok(())
}

/// Enabled effective admins other than `except`.
async fn other_admins(db: &impl ConnectionTrait, except: Uuid) -> anyhow::Result<u64> {
    user_entity::Entity::find()
        .filter(user_entity::Column::Id.ne(except))
        .filter(
            user_entity::Column::Admin
                .eq(true)
                .or(user_entity::Column::AdminFromGroups.eq(true)),
        )
        .filter(user_entity::Column::DisabledAt.is_null())
        .count(db)
        .await
        .context("counting admins")
}

/// Sets the manual admin flag and/or disables (re-enables) `target`. Refuses with
/// `last_admin` if the change would leave no enabled effective admin.
pub async fn update_flags(
    db: &DatabaseConnection,
    target: Uuid,
    admin: Option<bool>,
    disabled: Option<bool>,
) -> Result<user_entity::Model, ApiError> {
    let tx = db.begin().await.context("starting a transaction")?;
    lock_admin_changes(&tx).await?;
    let user = users::find(&tx, target).await?.ok_or(ApiError::NotFound)?;

    let now = crate::time::now();
    let new_admin = admin.unwrap_or(user.admin);
    let new_disabled_at = match disabled {
        Some(true) => user.disabled_at.or(Some(now)),
        Some(false) => None,
        None => user.disabled_at,
    };
    let was_enabled_admin = user.is_admin() && !user.is_disabled();
    let stays_enabled_admin = (new_admin || user.admin_from_groups) && new_disabled_at.is_none();
    if was_enabled_admin && !stays_enabled_admin && other_admins(&tx, target).await? == 0 {
        return Err(ApiError::LastAdmin);
    }

    user_entity::Entity::update_many()
        .col_expr(user_entity::Column::Admin, Expr::value(new_admin))
        .col_expr(
            user_entity::Column::DisabledAt,
            Expr::value(new_disabled_at),
        )
        .col_expr(user_entity::Column::UpdatedAt, Expr::value(now))
        .filter(user_entity::Column::Id.eq(target))
        .exec(&tx)
        .await
        .context("updating admin flags")?;
    if new_disabled_at.is_some() {
        // Signed out for good: re-enabling must not revive old sessions. Device
        // tokens stay (they are refused while disabled and work again afterwards).
        crate::auth::sessions::delete_all(&tx, target, None).await?;
    }
    let updated = users::find(&tx, target)
        .await?
        .context("user vanished during update")?;
    tx.commit().await.context("committing admin flags")?;
    Ok(updated)
}
