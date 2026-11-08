//! `devices`: desktop clients with a bearer token (`cld_…`), fixed expiry, soft revoke.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "devices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub os: String,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub created_at: DateTimeUtc,
    pub last_seen_at: Option<DateTimeUtc>,
    /// `None` = never expires.
    pub expires_at: Option<DateTimeUtc>,
    pub revoked_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
