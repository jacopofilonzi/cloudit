//! `scope_members`: a user a scope is shared with, and the role (`viewer`, `editor`,
//! `admin`). The owner is implicit (`scopes.owner_id`) and has no row.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_members")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    /// `viewer`, `editor` or `admin` (`MemberRole`).
    pub role: String,
    pub added_by: Uuid,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
