//! `chunks`: one stored chunk object of a scope (`scopes/{scope_id}/chunks/{hash}`),
//! inserted only after the object was stored and verified to decode to `hash`.
//! `size` is the uncompressed size, `stored_size` the object's (counted in
//! `scopes.stored_bytes`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chunks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub scope_id: Uuid,
    /// Lowercase hex SHA-256 of the uncompressed bytes.
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
    pub size: i32,
    pub stored_size: i64,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
