//! `snapshots`: one immutable version of a folder. `scope_id` repeats the folder's
//! scope (it references `manifests (scope_id, hash)`); `kind` is `normal`,
//! `conflict` or `restore` (`SnapshotKind`). Only `pinned`, `label` and
//! `conflict_resolved_at` ever change.

use cloudit_core::protocol::SnapshotKind;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "snapshots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub folder_id: Uuid,
    pub scope_id: Uuid,
    /// No foreign key: retention may delete the parent.
    pub parent_id: Option<Uuid>,
    pub kind: String,
    pub manifest_hash: String,
    pub file_count: i64,
    pub logical_size: i64,
    pub author_id: Uuid,
    pub device_id: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub pinned: bool,
    pub label: Option<String>,
    /// No foreign key: retention may delete the source.
    pub restored_from: Option<Uuid>,
    pub conflict_resolved_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// The kind; an unknown stored value reads as `normal`.
    #[must_use]
    pub fn kind(&self) -> SnapshotKind {
        SnapshotKind::parse(&self.kind).unwrap_or(SnapshotKind::Normal)
    }
}
