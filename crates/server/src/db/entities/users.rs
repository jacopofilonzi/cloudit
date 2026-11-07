//! `users`: one account. `email` is stored trimmed and lowercased (unique).
//! `password_hash` is null for accounts that only sign in through an identity provider
//! (milestone 3c). Effective admin = `admin || admin_from_groups`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub email: String,
    pub email_verified: bool,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub admin: bool,
    pub admin_from_groups: bool,
    pub disabled_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Manual or group-derived admin.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.admin || self.admin_from_groups
    }

    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }
}
