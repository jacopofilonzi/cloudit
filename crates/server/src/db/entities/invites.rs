//! `invites`: single-use registration invites created by admins.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invites")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub created_by: Uuid,
    /// If set, registration must use this (normalized) email.
    pub email: Option<String>,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
    pub used_by: Option<Uuid>,
    pub revoked_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
