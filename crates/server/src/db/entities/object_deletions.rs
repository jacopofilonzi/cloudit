//! `object_deletions`: an object whose `chunks` / `manifests` row the garbage collector
//! deleted and whose object it still has to delete (migration 000009; protocol in
//! `sync::objects`). `kind` is `chunk` or `manifest`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "object_deletions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
