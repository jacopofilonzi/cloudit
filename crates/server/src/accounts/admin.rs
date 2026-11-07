//! Admin changes to an account (`admin` flag, disable, approve / reject) with the
//! last-admin guard.
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
//!
//! Approval transitions are one conditional UPDATE each (`… WHERE id = ? AND
//! approval_status IN (…)`), so concurrent decisions can't interleave. Rejecting
//! removes an approved account from the admins, so it takes the same lock and guard;
//! it also deletes the web sessions (device tokens are refused while rejected).

use anyhow::Context as _;
use cloudit_core::protocol::ApprovalStatus;
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

/// Enabled, approved effective admins other than `except`.
async fn other_admins(db: &impl ConnectionTrait, except: Uuid) -> anyhow::Result<u64> {
    user_entity::Entity::find()
        .filter(user_entity::Column::Id.ne(except))
        .filter(
            user_entity::Column::Admin
                .eq(true)
                .or(user_entity::Column::AdminFromGroups.eq(true)),
        )
        .filter(user_entity::Column::DisabledAt.is_null())
        .filter(user_entity::Column::ApprovalStatus.eq(ApprovalStatus::Approved.as_str()))
        .count(db)
        .await
        .context("counting admins")
}

/// Sets `admin_from_groups` after a sign-in through an OIDC provider (see
/// `oauth::complete`). Clearing it is skipped (and logged) when it would leave no
/// enabled effective admin, the same guard as [`update_flags`]: an identity provider
/// dropping a group must not lock everyone out of administration.
pub async fn set_admin_from_groups(
    db: &DatabaseConnection,
    target: Uuid,
    value: bool,
) -> anyhow::Result<()> {
    // Most sign-ins change nothing: skip the lock then.
    if users::find(db, target)
        .await?
        .is_none_or(|u| u.admin_from_groups == value)
    {
        return Ok(());
    }
    let tx = db.begin().await.context("starting a transaction")?;
    lock_admin_changes(&tx).await?;
    let Some(user) = users::find(&tx, target).await? else {
        return Ok(());
    };
    if user.admin_from_groups == value {
        return Ok(());
    }
    if !value && !user.admin && user.is_active() && other_admins(&tx, target).await? == 0 {
        tracing::warn!(
            user_id = %target,
            "keeping group-derived admin: removing it would leave no enabled admin"
        );
        return Ok(());
    }
    user_entity::Entity::update_many()
        .col_expr(user_entity::Column::AdminFromGroups, Expr::value(value))
        .col_expr(
            user_entity::Column::UpdatedAt,
            Expr::value(crate::time::now()),
        )
        .filter(user_entity::Column::Id.eq(target))
        .exec(&tx)
        .await
        .context("updating admin_from_groups")?;
    tx.commit().await.context("committing admin_from_groups")?;
    tracing::info!(user_id = %target, admin_from_groups = value, "group-derived admin changed");
    Ok(())
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
    let was_enabled_admin = user.is_admin() && user.is_active();
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

/// Result of an approval transition.
pub struct Decision {
    pub user: user_entity::Model,
    /// `false` when the account already was in the target state (nothing written).
    pub changed: bool,
}

/// One conditional UPDATE: `target` moves to `to` only from one of `from`, recording
/// the deciding admin. `true` if the row changed.
async fn transition(
    db: &impl ConnectionTrait,
    target: Uuid,
    from: &[ApprovalStatus],
    to: ApprovalStatus,
    by: Uuid,
) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let result = user_entity::Entity::update_many()
        .col_expr(
            user_entity::Column::ApprovalStatus,
            Expr::value(to.as_str()),
        )
        .col_expr(user_entity::Column::ApprovalDecidedAt, Expr::value(now))
        .col_expr(user_entity::Column::ApprovalDecidedBy, Expr::value(by))
        .col_expr(user_entity::Column::UpdatedAt, Expr::value(now))
        .filter(user_entity::Column::Id.eq(target))
        .filter(user_entity::Column::ApprovalStatus.is_in(from.iter().map(|s| s.as_str())))
        .exec(db)
        .await
        .context("changing the approval status")?;
    Ok(result.rows_affected == 1)
}

/// Approves a pending account, or lifts the ban of a rejected one. Approving can't
/// remove an admin, so it needs no lock. Approving an approved account changes nothing.
pub async fn approve(
    db: &DatabaseConnection,
    target: Uuid,
    by: Uuid,
) -> Result<Decision, ApiError> {
    let changed = transition(
        db,
        target,
        &[ApprovalStatus::Pending, ApprovalStatus::Rejected],
        ApprovalStatus::Approved,
        by,
    )
    .await?;
    let user = users::find(db, target).await?.ok_or(ApiError::NotFound)?;
    Ok(Decision { user, changed })
}

/// Rejects (bans) a pending or approved account and deletes its web sessions in the
/// same transaction; its device tokens are refused from then on. `forbidden` for the
/// caller's own account, `last_admin` if it would leave no enabled approved admin.
/// Rejecting a rejected account changes nothing.
pub async fn reject(db: &DatabaseConnection, target: Uuid, by: Uuid) -> Result<Decision, ApiError> {
    if target == by {
        return Err(ApiError::Forbidden);
    }
    let tx = db.begin().await.context("starting a transaction")?;
    lock_admin_changes(&tx).await?;
    let user = users::find(&tx, target).await?.ok_or(ApiError::NotFound)?;
    if user.is_admin() && user.is_active() && other_admins(&tx, target).await? == 0 {
        return Err(ApiError::LastAdmin);
    }
    let changed = transition(
        &tx,
        target,
        &[ApprovalStatus::Pending, ApprovalStatus::Approved],
        ApprovalStatus::Rejected,
        by,
    )
    .await?;
    if changed {
        crate::auth::sessions::delete_all(&tx, target, None).await?;
    }
    let user = users::find(&tx, target)
        .await?
        .context("user vanished during reject")?;
    tx.commit().await.context("committing a rejection")?;
    Ok(Decision { user, changed })
}
