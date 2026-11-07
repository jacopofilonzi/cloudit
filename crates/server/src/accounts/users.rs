//! The `users` table: creation, lookups, profile and password updates, admin listing.
//! Admin flag and disable changes live in [`super::admin`] (they need the last-admin
//! guard).

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, DbErr, EntityTrait as _,
    PaginatorTrait as _, QueryFilter as _, QueryOrder as _, QuerySelect as _, SqlErr,
};
use uuid::Uuid;

use crate::db::entities::users;
use crate::errors::ApiError;

/// A new account.
pub struct NewUser {
    /// Already normalized (`validate::email`).
    pub email: String,
    pub email_verified: bool,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub admin: bool,
}

/// `true` if `err` is a unique-constraint violation (on either backend).
pub fn is_unique_violation(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

/// Inserts a user; `email_taken` if the email is in use.
pub async fn insert(db: &impl ConnectionTrait, new: NewUser) -> Result<users::Model, ApiError> {
    let now = crate::time::now();
    let model = users::ActiveModel {
        id: Set(Uuid::now_v7()),
        email: Set(new.email),
        email_verified: Set(new.email_verified),
        display_name: Set(new.display_name),
        password_hash: Set(new.password_hash),
        admin: Set(new.admin),
        admin_from_groups: Set(false),
        disabled_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match users::Entity::insert(model).exec_with_returning(db).await {
        Ok(user) => Ok(user),
        Err(err) if is_unique_violation(&err) => Err(ApiError::EmailTaken),
        Err(err) => Err(anyhow::Error::new(err).context("inserting a user").into()),
    }
}

pub async fn find(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<Option<users::Model>> {
    users::Entity::find_by_id(id)
        .one(db)
        .await
        .context("loading a user")
}

/// Looks up by normalized email.
pub async fn find_by_email(
    db: &impl ConnectionTrait,
    email: &str,
) -> anyhow::Result<Option<users::Model>> {
    users::Entity::find()
        .filter(users::Column::Email.eq(email))
        .one(db)
        .await
        .context("loading a user by email")
}

/// `true` if at least one account exists.
pub async fn any_exists(db: &impl ConnectionTrait) -> anyhow::Result<bool> {
    let first = users::Entity::find()
        .select_only()
        .column(users::Column::Id)
        .limit(1)
        .into_tuple::<Uuid>()
        .one(db)
        .await
        .context("checking for users")?;
    Ok(first.is_some())
}

pub async fn set_password(
    db: &impl ConnectionTrait,
    id: Uuid,
    password_hash: String,
) -> anyhow::Result<()> {
    users::Entity::update_many()
        .col_expr(users::Column::PasswordHash, Expr::value(password_hash))
        .col_expr(users::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(users::Column::Id.eq(id))
        .exec(db)
        .await
        .context("setting a password")?;
    Ok(())
}

/// Updates the display name and/or the email (a changed email becomes unverified);
/// `email_taken` if the new email is in use.
pub async fn update_profile(
    db: &impl ConnectionTrait,
    id: Uuid,
    display_name: Option<String>,
    email: Option<String>,
) -> Result<(), ApiError> {
    let mut update = users::Entity::update_many()
        .col_expr(users::Column::UpdatedAt, Expr::value(crate::time::now()))
        .filter(users::Column::Id.eq(id));
    if let Some(name) = display_name {
        update = update.col_expr(users::Column::DisplayName, Expr::value(name));
    }
    if let Some(email) = email {
        update = update
            .col_expr(users::Column::Email, Expr::value(email))
            .col_expr(users::Column::EmailVerified, Expr::value(false));
    }
    match update.exec(db).await {
        Ok(_) => Ok(()),
        Err(err) if is_unique_violation(&err) => Err(ApiError::EmailTaken),
        Err(err) => Err(anyhow::Error::new(err).context("updating a profile").into()),
    }
}

/// Sets `admin_from_groups` of a just-created account (no last-admin guard needed: it
/// only grants). Changes after a sign-in go through `admin::set_admin_from_groups`.
pub async fn grant_admin_from_groups(db: &impl ConnectionTrait, id: Uuid) -> anyhow::Result<()> {
    users::Entity::update_many()
        .col_expr(users::Column::AdminFromGroups, Expr::value(true))
        .filter(users::Column::Id.eq(id))
        .exec(db)
        .await
        .context("setting admin_from_groups")?;
    Ok(())
}

/// A page of accounts, oldest first, and the total count.
pub async fn list(
    db: &impl ConnectionTrait,
    limit: u64,
    offset: u64,
) -> anyhow::Result<(Vec<users::Model>, u64)> {
    let users = users::Entity::find()
        .order_by_asc(users::Column::CreatedAt)
        .order_by_asc(users::Column::Id)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
        .context("listing users")?;
    let total = users::Entity::find()
        .count(db)
        .await
        .context("counting users")?;
    Ok((users, total))
}
