//! `sessions`: web sessions (cookie `cloudit_session`), sliding expiry.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub user_id: Uuid,
    pub created_at: DateTimeUtc,
    pub last_seen_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub user_agent: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
