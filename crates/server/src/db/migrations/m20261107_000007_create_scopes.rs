//! Scopes, their members and folders: `scopes`, `scope_members`, `folders`.
//!
//! Names are unique, ignoring case, among the siblings outside the trash (a user's
//! scopes, a scope's folders). Partial unique indexes can't be written with the
//! portable schema builder, so each table stores `name_key` (the lowercased name) and
//! `active_key` (= `name_key`, or NULL while in the trash) with a UNIQUE index on
//! `(parent, active_key)`: NULLs never collide on SQLite or PostgreSQL, so trashed
//! items don't block a name, and the database itself rejects a duplicate (no
//! check-then-insert race). Restoring sets `active_key` back and fails the same way.
//!
//! `folders.head_snapshot_id` has no foreign key yet: the `snapshots` table comes with
//! the sync protocol (milestone 4b).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
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
    DeletedBy,
}

#[derive(DeriveIden)]
enum ScopeMembers {
    Table,
    ScopeId,
    UserId,
    Role,
    AddedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Folders {
    Table,
    Id,
    ScopeId,
    Name,
    NameKey,
    ActiveKey,
    HeadSnapshotId,
    RetentionMaxVersions,
    RetentionMaxPerDay,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
    DeletedBy,
}

fn timestamp(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .timestamp_with_time_zone()
        .not_null()
        .to_owned()
}

fn timestamp_null(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .timestamp_with_time_zone()
        .null()
        .to_owned()
}

fn fk(
    name: &str,
    table: impl IntoIden,
    col: impl IntoIden,
    to: impl IntoIden,
    to_col: impl IntoIden,
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .name(name)
        .from(table, col)
        .to(to, to_col)
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Scopes::Table)
                    .col(ColumnDef::new(Scopes::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Scopes::OwnerId).uuid().not_null())
                    .col(ColumnDef::new(Scopes::Name).text().not_null())
                    .col(ColumnDef::new(Scopes::NameKey).text().not_null())
                    .col(ColumnDef::new(Scopes::ActiveKey).text().null())
                    .col(timestamp(Scopes::CreatedAt))
                    .col(timestamp(Scopes::UpdatedAt))
                    .col(timestamp_null(Scopes::DeletedAt))
                    .col(ColumnDef::new(Scopes::DeletedBy).uuid().null())
                    .foreign_key(&mut fk(
                        "fk_scopes_owner",
                        Scopes::Table,
                        Scopes::OwnerId,
                        Users::Table,
                        Users::Id,
                    ))
                    .foreign_key(&mut fk(
                        "fk_scopes_deleted_by",
                        Scopes::Table,
                        Scopes::DeletedBy,
                        Users::Table,
                        Users::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_scopes_owner_active_key")
                    .table(Scopes::Table)
                    .col(Scopes::OwnerId)
                    .col(Scopes::ActiveKey)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ScopeMembers::Table)
                    .col(ColumnDef::new(ScopeMembers::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(ScopeMembers::UserId).uuid().not_null())
                    .col(ColumnDef::new(ScopeMembers::Role).text().not_null())
                    .col(ColumnDef::new(ScopeMembers::AddedBy).uuid().not_null())
                    .col(timestamp(ScopeMembers::CreatedAt))
                    .col(timestamp(ScopeMembers::UpdatedAt))
                    .primary_key(
                        Index::create()
                            .col(ScopeMembers::ScopeId)
                            .col(ScopeMembers::UserId),
                    )
                    .foreign_key(&mut fk(
                        "fk_scope_members_scope",
                        ScopeMembers::Table,
                        ScopeMembers::ScopeId,
                        Scopes::Table,
                        Scopes::Id,
                    ))
                    .foreign_key(&mut fk(
                        "fk_scope_members_user",
                        ScopeMembers::Table,
                        ScopeMembers::UserId,
                        Users::Table,
                        Users::Id,
                    ))
                    .foreign_key(&mut fk(
                        "fk_scope_members_added_by",
                        ScopeMembers::Table,
                        ScopeMembers::AddedBy,
                        Users::Table,
                        Users::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_scope_members_user_id")
                    .table(ScopeMembers::Table)
                    .col(ScopeMembers::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Folders::Table)
                    .col(ColumnDef::new(Folders::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Folders::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(Folders::Name).text().not_null())
                    .col(ColumnDef::new(Folders::NameKey).text().not_null())
                    .col(ColumnDef::new(Folders::ActiveKey).text().null())
                    .col(ColumnDef::new(Folders::HeadSnapshotId).uuid().null())
                    .col(
                        ColumnDef::new(Folders::RetentionMaxVersions)
                            .integer()
                            .null(),
                    )
                    .col(ColumnDef::new(Folders::RetentionMaxPerDay).integer().null())
                    .col(ColumnDef::new(Folders::CreatedBy).uuid().not_null())
                    .col(timestamp(Folders::CreatedAt))
                    .col(timestamp(Folders::UpdatedAt))
                    .col(timestamp_null(Folders::DeletedAt))
                    .col(ColumnDef::new(Folders::DeletedBy).uuid().null())
                    .foreign_key(&mut fk(
                        "fk_folders_scope",
                        Folders::Table,
                        Folders::ScopeId,
                        Scopes::Table,
                        Scopes::Id,
                    ))
                    .foreign_key(&mut fk(
                        "fk_folders_created_by",
                        Folders::Table,
                        Folders::CreatedBy,
                        Users::Table,
                        Users::Id,
                    ))
                    .foreign_key(&mut fk(
                        "fk_folders_deleted_by",
                        Folders::Table,
                        Folders::DeletedBy,
                        Users::Table,
                        Users::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("uq_folders_scope_active_key")
                    .table(Folders::Table)
                    .col(Folders::ScopeId)
                    .col(Folders::ActiveKey)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
