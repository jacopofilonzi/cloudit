//! `settings`: server settings editable by admins, one JSON value per key.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    /// JSON text.
    pub value: String,
    pub updated_at: DateTimeUtc,
    pub updated_by: Option<Uuid>,
}

impl ActiveModelBehavior for ActiveModel {}
