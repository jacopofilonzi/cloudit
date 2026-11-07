//! The sync protocol: `chunks`, `manifests`, `snapshots`, `snapshot_chunks`, the usage
//! counter `scopes.stored_bytes` and the per-user `users.quota_bytes`.
//!
//! - `chunks` / `manifests`: one row per stored object of a scope (PK `(scope_id,
//!   hash)`), inserted only after the object was stored **and** verified; hashes are
//!   lowercase hex text. `stored_size` is the object's size (what quotas count).
//! - `snapshots`: `scope_id` repeats the folder's scope so `(scope_id, manifest_hash)`
//!   can reference `manifests`. `parent_id` and `restored_from` have no foreign key:
//!   retention deletes old snapshots that newer ones still name.
//! - `snapshot_chunks`: the distinct chunk hashes of each snapshot, the reference set
//!   of the chunk garbage collector; `(scope_id, hash)` references `chunks`, so a chunk
//!   row can't be deleted while a snapshot uses it.
//! - `scopes.stored_bytes` (NOT NULL, default 0: no object existed before) and
//!   `users.quota_bytes` (NULL = the server default).
//!
//! `folders.head_snapshot_id` (migration 000007) gets no foreign key: SQLite can't add
//! one to an existing table without rebuilding it. Only the commit and restore
//! statements set it, always to a snapshot of the same folder in the same transaction,
//! and retention never deletes a head.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    QuotaBytes,
}

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Scopes {
    Table,
    Id,
    StoredBytes,
}

#[derive(DeriveIden)]
enum Folders {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Chunks {
    Table,
    ScopeId,
    Hash,
    Size,
    StoredSize,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Manifests {
    Table,
    ScopeId,
    Hash,
    StoredSize,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Snapshots {
    Table,
    Id,
    FolderId,
    ScopeId,
    ParentId,
    Kind,
    ManifestHash,
    FileCount,
    LogicalSize,
    AuthorId,
    DeviceId,
    CreatedAt,
    Pinned,
    Label,
    RestoredFrom,
    ConflictResolvedAt,
}

#[derive(DeriveIden)]
enum SnapshotChunks {
    Table,
    SnapshotId,
    ScopeId,
    Hash,
}

fn timestamp(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .timestamp_with_time_zone()
        .not_null()
        .to_owned()
}

fn add(table: impl IntoIden, col: &mut ColumnDef) -> TableAlterStatement {
    Table::alter().table(table).add_column(col).to_owned()
}

fn index(name: &str, table: impl IntoIden, cols: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(table);
    for col in cols {
        index.col(Alias::new(*col));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(add(
                Scopes::Table,
                ColumnDef::new(Scopes::StoredBytes)
                    .big_integer()
                    .not_null()
                    .default(0),
            ))
            .await?;
        manager
            .alter_table(add(
                Users::Table,
                ColumnDef::new(Users::QuotaBytes).big_integer().null(),
            ))
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Chunks::Table)
                    .col(ColumnDef::new(Chunks::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(Chunks::Hash).text().not_null())
                    .col(ColumnDef::new(Chunks::Size).integer().not_null())
                    .col(ColumnDef::new(Chunks::StoredSize).big_integer().not_null())
                    .col(timestamp(Chunks::CreatedAt))
                    .primary_key(Index::create().col(Chunks::ScopeId).col(Chunks::Hash))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_chunks_scope")
                            .from(Chunks::Table, Chunks::ScopeId)
                            .to(Scopes::Table, Scopes::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Manifests::Table)
                    .col(ColumnDef::new(Manifests::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(Manifests::Hash).text().not_null())
                    .col(
                        ColumnDef::new(Manifests::StoredSize)
                            .big_integer()
                            .not_null(),
                    )
                    .col(timestamp(Manifests::CreatedAt))
                    .primary_key(Index::create().col(Manifests::ScopeId).col(Manifests::Hash))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_manifests_scope")
                            .from(Manifests::Table, Manifests::ScopeId)
                            .to(Scopes::Table, Scopes::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Snapshots::Table)
                    .col(
                        ColumnDef::new(Snapshots::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Snapshots::FolderId).uuid().not_null())
                    .col(ColumnDef::new(Snapshots::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(Snapshots::ParentId).uuid().null())
                    .col(ColumnDef::new(Snapshots::Kind).text().not_null())
                    .col(ColumnDef::new(Snapshots::ManifestHash).text().not_null())
                    .col(
                        ColumnDef::new(Snapshots::FileCount)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Snapshots::LogicalSize)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Snapshots::AuthorId).uuid().not_null())
                    .col(ColumnDef::new(Snapshots::DeviceId).uuid().null())
                    .col(timestamp(Snapshots::CreatedAt))
                    .col(
                        ColumnDef::new(Snapshots::Pinned)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(Snapshots::Label).text().null())
                    .col(ColumnDef::new(Snapshots::RestoredFrom).uuid().null())
                    .col(
                        ColumnDef::new(Snapshots::ConflictResolvedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshots_folder")
                            .from(Snapshots::Table, Snapshots::FolderId)
                            .to(Folders::Table, Folders::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshots_manifest")
                            .from(
                                Snapshots::Table,
                                (Snapshots::ScopeId, Snapshots::ManifestHash),
                            )
                            .to(Manifests::Table, (Manifests::ScopeId, Manifests::Hash)),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshots_author")
                            .from(Snapshots::Table, Snapshots::AuthorId)
                            .to(Users::Table, Users::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshots_device")
                            .from(Snapshots::Table, Snapshots::DeviceId)
                            .to(Devices::Table, Devices::Id),
                    )
                    .to_owned(),
            )
            .await?;
        // History pages (newest first by id, UUID v7) and the FK side of manifests.
        manager
            .create_index(index(
                "idx_snapshots_folder_id",
                Snapshots::Table,
                &["folder_id", "id"],
            ))
            .await?;
        manager
            .create_index(index(
                "idx_snapshots_scope_manifest",
                Snapshots::Table,
                &["scope_id", "manifest_hash"],
            ))
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(SnapshotChunks::Table)
                    .col(ColumnDef::new(SnapshotChunks::SnapshotId).uuid().not_null())
                    .col(ColumnDef::new(SnapshotChunks::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(SnapshotChunks::Hash).text().not_null())
                    .primary_key(
                        Index::create()
                            .col(SnapshotChunks::SnapshotId)
                            .col(SnapshotChunks::Hash),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshot_chunks_snapshot")
                            .from(SnapshotChunks::Table, SnapshotChunks::SnapshotId)
                            .to(Snapshots::Table, Snapshots::Id),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_snapshot_chunks_chunk")
                            .from(
                                SnapshotChunks::Table,
                                (SnapshotChunks::ScopeId, SnapshotChunks::Hash),
                            )
                            .to(Chunks::Table, (Chunks::ScopeId, Chunks::Hash)),
                    )
                    .to_owned(),
            )
            .await?;
        // "Is this chunk still referenced?" (garbage collector, FK checks on delete).
        manager
            .create_index(index(
                "idx_snapshot_chunks_scope_hash",
                SnapshotChunks::Table,
                &["scope_id", "hash"],
            ))
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::sea_query::Query;
    use sea_orm::{ConnectionTrait as _, EntityTrait as _};
    use sea_orm_migration::MigratorTrait as _;
    use uuid::Uuid;

    use crate::db::entities::{scopes, users};
    use crate::db::migrations::Migrator;
    use crate::db::testing::TestDb;

    #[derive(sea_orm::DeriveIden)]
    enum Users {
        Table,
        Id,
        Email,
        EmailVerified,
        DisplayName,
        PasswordHash,
        Admin,
        AdminFromGroups,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(sea_orm::DeriveIden)]
    enum Scopes {
        Table,
        Id,
        OwnerId,
        Name,
        NameKey,
        ActiveKey,
        CreatedAt,
        UpdatedAt,
        DeletedAt,
    }

    /// Accounts and scopes (one in the trash) from before this migration: no quota,
    /// nothing stored.
    #[tokio::test]
    async fn existing_rows_get_the_defaults() {
        let test = TestDb::empty().await;
        Migrator::up(&test.db, Some(7)).await.unwrap();

        let now = crate::time::now();
        let user = Uuid::now_v7();
        let stmt = Query::insert()
            .into_table(Users::Table)
            .columns([
                Users::Id,
                Users::Email,
                Users::EmailVerified,
                Users::DisplayName,
                Users::PasswordHash,
                Users::Admin,
                Users::AdminFromGroups,
                Users::CreatedAt,
                Users::UpdatedAt,
            ])
            .values_panic([
                user.into(),
                "alice@example.com".into(),
                true.into(),
                "Alice".into(),
                Some("$argon2id$v=19$x".to_owned()).into(),
                true.into(),
                false.into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        test.db.execute(&stmt).await.unwrap();
        for (name, deleted_at) in [("Games", None), ("Old", Some(now))] {
            let stmt = Query::insert()
                .into_table(Scopes::Table)
                .columns([
                    Scopes::Id,
                    Scopes::OwnerId,
                    Scopes::Name,
                    Scopes::NameKey,
                    Scopes::ActiveKey,
                    Scopes::CreatedAt,
                    Scopes::UpdatedAt,
                    Scopes::DeletedAt,
                ])
                .values_panic([
                    Uuid::now_v7().into(),
                    user.into(),
                    name.into(),
                    name.to_lowercase().into(),
                    deleted_at.is_none().then(|| name.to_lowercase()).into(),
                    now.into(),
                    now.into(),
                    deleted_at.into(),
                ])
                .to_owned();
            test.db.execute(&stmt).await.unwrap();
        }

        Migrator::up(&test.db, None).await.unwrap();

        let all = scopes::Entity::find().all(&test.db).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|s| s.stored_bytes == 0));
        let alice = users::Entity::find_by_id(user)
            .one(&test.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alice.quota_bytes, None);
        assert_eq!(alice.email, "alice@example.com");
    }
}
