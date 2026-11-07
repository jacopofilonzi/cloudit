//! `manifests`: one stored manifest object of a scope
//! (`scopes/{scope_id}/manifests/{hash}`, zstd canonical JSON), inserted after the
//! object was stored and decoded. `stored_size` is counted in `scopes.stored_bytes`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "manifests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: Uuid,
    /// Lowercase hex SHA-256 of the canonical JSON.
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
    pub stored_size: i64,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
