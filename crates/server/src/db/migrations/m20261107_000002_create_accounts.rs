//! Accounts: `users`, web `sessions`, the first-run `setup_state` row and `settings`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    EmailVerified,
    DisplayName,
    PasswordHash,
    Admin,
    AdminFromGroups,
    DisabledAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    TokenHash,
    UserId,
    CreatedAt,
    LastSeenAt,
    ExpiresAt,
    UserAgent,
}

#[derive(DeriveIden)]
enum SetupState {
    Table,
    Id,
    Token,
    CreatedAt,
    CompletedAt,
}

#[derive(DeriveIden)]
enum Settings {
    Table,
    Key,
    Value,
    UpdatedAt,
    UpdatedBy,
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .col(ColumnDef::new(Users::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Users::Email).text().not_null().unique_key())
                    .col(
                        ColumnDef::new(Users::EmailVerified)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(Users::DisplayName).text().not_null())
                    .col(ColumnDef::new(Users::PasswordHash).text().null())
                    .col(
                        ColumnDef::new(Users::Admin)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Users::AdminFromGroups)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(timestamp_null(Users::DisabledAt))
                    .col(timestamp(Users::CreatedAt))
                    .col(timestamp(Users::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .col(ColumnDef::new(Sessions::Id).uuid().not_null().primary_key())
                    .col(
                        ColumnDef::new(Sessions::TokenHash)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Sessions::UserId).uuid().not_null())
                    .col(timestamp(Sessions::CreatedAt))
                    .col(timestamp(Sessions::LastSeenAt))
                    .col(timestamp(Sessions::ExpiresAt))
                    .col(ColumnDef::new(Sessions::UserAgent).text().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_sessions_user")
                            .from(Sessions::Table, Sessions::UserId)
                            .to(Users::Table, Users::Id),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_user_id")
                    .table(Sessions::Table)
                    .col(Sessions::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(SetupState::Table)
                    .col(
                        ColumnDef::new(SetupState::Id)
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SetupState::Token).text().null())
                    .col(timestamp(SetupState::CreatedAt))
                    .col(timestamp_null(SetupState::CompletedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Settings::Table)
                    .col(
                        ColumnDef::new(Settings::Key)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Settings::Value).text().not_null())
                    .col(timestamp(Settings::UpdatedAt))
                    .col(ColumnDef::new(Settings::UpdatedBy).uuid().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_settings_updated_by")
                            .from(Settings::Table, Settings::UpdatedBy)
                            .to(Users::Table, Users::Id),
                    )
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
