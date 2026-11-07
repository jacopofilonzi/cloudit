//! `scopes`: a user-named group of folders, owned by one user. `name_key` is the
//! lowercased name; `active_key` equals it outside the trash and is NULL inside, under
//! a UNIQUE `(owner_id, active_key)` index (see migration 000007). `stored_bytes` is
//! the usage counter (migration 000008, `sync::usage`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "scopes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub name_key: String,
    pub active_key: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    /// Set while in the trash.
    pub deleted_at: Option<DateTimeUtc>,
    pub deleted_by: Option<Uuid>,
    /// Stored bytes of the scope's chunks and manifests (rows of `chunks` and
    /// `manifests`), maintained in the transactions that insert or delete them.
    pub stored_bytes: i64,
}

impl ActiveModelBehavior for ActiveModel {}
