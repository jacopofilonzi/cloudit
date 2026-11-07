//! `snapshot_chunks`: the distinct chunk hashes a snapshot references (the garbage
//! collector's reference set). `(scope_id, hash)` references `chunks`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "snapshot_chunks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub snapshot_id: Uuid,
    pub scope_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
}

impl ActiveModelBehavior for ActiveModel {}
