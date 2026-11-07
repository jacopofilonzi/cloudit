//! Single-use and long-lived tokens: `invites`, `password_resets`, `devices` and
//! `device_auth_codes`. Only SHA-256 hashes of the tokens are stored.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Invites {
    Table,
    Id,
    TokenHash,
    CreatedBy,
    Email,
    CreatedAt,
    ExpiresAt,
    UsedAt,
    UsedBy,
    RevokedAt,
}

#[derive(DeriveIden)]
enum PasswordResets {
    Table,
    Id,
    TokenHash,
    UserId,
    CreatedBy,
    CreatedAt,
    ExpiresAt,
    UsedAt,
}

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
    UserId,
    Name,
    Os,
    TokenHash,
    CreatedAt,
    LastSeenAt,
    ExpiresAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum DeviceAuthCodes {
    Table,
    Id,
    CodeHash,
    UserId,
    CodeChallenge,
    RedirectUri,
    DeviceName,
    Os,
    TokenTtlSecs,
    CreatedAt,
    ExpiresAt,
    UsedAt,
}

fn id(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .uuid()
        .not_null()
        .primary_key()
        .to_owned()
}

fn hash(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .text()
        .not_null()
        .unique_key()
        .to_owned()
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

fn user_fk(name: &str, table: impl IntoIden, col: impl IntoIden) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .name(name)
        .from(table, col)
        .to(Users::Table, Users::Id)
        .to_owned()
}

async fn index(
    manager: &SchemaManager<'_>,
    name: &str,
    table: impl IntoIden,
    col: impl IntoIden,
) -> Result<(), DbErr> {
    manager
        .create_index(Index::create().name(name).table(table).col(col).to_owned())
        .await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Invites::Table)
                    .col(id(Invites::Id))
                    .col(hash(Invites::TokenHash))
                    .col(ColumnDef::new(Invites::CreatedBy).uuid().not_null())
                    .col(ColumnDef::new(Invites::Email).text().null())
                    .col(timestamp(Invites::CreatedAt))
                    .col(timestamp(Invites::ExpiresAt))
                    .col(timestamp_null(Invites::UsedAt))
                    .col(ColumnDef::new(Invites::UsedBy).uuid().null())
                    .col(timestamp_null(Invites::RevokedAt))
                    .foreign_key(&mut user_fk(
                        "fk_invites_created_by",
                        Invites::Table,
                        Invites::CreatedBy,
                    ))
                    .foreign_key(&mut user_fk(
                        "fk_invites_used_by",
                        Invites::Table,
                        Invites::UsedBy,
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(PasswordResets::Table)
                    .col(id(PasswordResets::Id))
                    .col(hash(PasswordResets::TokenHash))
                    .col(ColumnDef::new(PasswordResets::UserId).uuid().not_null())
                    .col(ColumnDef::new(PasswordResets::CreatedBy).uuid().null())
                    .col(timestamp(PasswordResets::CreatedAt))
                    .col(timestamp(PasswordResets::ExpiresAt))
                    .col(timestamp_null(PasswordResets::UsedAt))
                    .foreign_key(&mut user_fk(
                        "fk_password_resets_user",
                        PasswordResets::Table,
                        PasswordResets::UserId,
                    ))
                    .foreign_key(&mut user_fk(
                        "fk_password_resets_created_by",
                        PasswordResets::Table,
                        PasswordResets::CreatedBy,
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Devices::Table)
                    .col(id(Devices::Id))
                    .col(ColumnDef::new(Devices::UserId).uuid().not_null())
                    .col(ColumnDef::new(Devices::Name).text().not_null())
                    .col(ColumnDef::new(Devices::Os).text().not_null())
                    .col(hash(Devices::TokenHash))
                    .col(timestamp(Devices::CreatedAt))
                    .col(timestamp_null(Devices::LastSeenAt))
                    .col(timestamp(Devices::ExpiresAt))
                    .col(timestamp_null(Devices::RevokedAt))
                    .foreign_key(&mut user_fk(
                        "fk_devices_user",
                        Devices::Table,
                        Devices::UserId,
                    ))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "idx_devices_user_id",
            Devices::Table,
            Devices::UserId,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(DeviceAuthCodes::Table)
                    .col(id(DeviceAuthCodes::Id))
                    .col(hash(DeviceAuthCodes::CodeHash))
                    .col(ColumnDef::new(DeviceAuthCodes::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(DeviceAuthCodes::CodeChallenge)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DeviceAuthCodes::RedirectUri)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DeviceAuthCodes::DeviceName)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(DeviceAuthCodes::Os).text().not_null())
                    .col(
                        ColumnDef::new(DeviceAuthCodes::TokenTtlSecs)
                            .big_integer()
                            .not_null(),
                    )
                    .col(timestamp(DeviceAuthCodes::CreatedAt))
                    .col(timestamp(DeviceAuthCodes::ExpiresAt))
                    .col(timestamp_null(DeviceAuthCodes::UsedAt))
                    .foreign_key(&mut user_fk(
                        "fk_device_auth_codes_user",
                        DeviceAuthCodes::Table,
                        DeviceAuthCodes::UserId,
                    ))
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
