//! `password_resets`: single-use reset links (24 h), created by an admin or the CLI.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "password_resets")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub user_id: Uuid,
    /// `None` when created by `cloudit-server admin reset-password`.
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
