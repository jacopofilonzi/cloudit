//! `jobs`: one row per background job (migration 000009): an immediate run requested
//! by an admin, and the last start by any instance (see `crate::jobs`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "jobs")]
pub struct Model {
    /// `JobName::as_str`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    pub requested_at: Option<DateTimeUtc>,
    pub last_started_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
