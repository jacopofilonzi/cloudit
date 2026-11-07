//! `folders`: a synced unit inside a scope. Names work like scope names
//! (`name_key`, `active_key`, UNIQUE `(scope_id, active_key)`). Retention overrides are
//! NULL for the server default, 0 for unlimited. `head_snapshot_id` has no foreign key
//! until the snapshots table exists (milestone 4b).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "folders")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub scope_id: Uuid,
    pub name: String,
    pub name_key: String,
    pub active_key: Option<String>,
    pub head_snapshot_id: Option<Uuid>,
    pub retention_max_versions: Option<i32>,
    pub retention_max_per_day: Option<i32>,
    pub created_by: Uuid,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    /// Set while in the trash.
    pub deleted_at: Option<DateTimeUtc>,
    pub deleted_by: Option<Uuid>,
}

impl ActiveModelBehavior for ActiveModel {}
