//! Identity providers configured by admins: `auth_providers`. The id is a durable text
//! slug (identities reference it, callback URLs contain it), never reused: deleting is
//! soft (`deleted_at`). The client secret is stored encrypted (`crypto::SecretBox`).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum AuthProviders {
    Table,
    Id,
    Kind,
    DisplayName,
    Icon,
    IssuerUrl,
    Tenant,
    ClientId,
    ClientSecretEnc,
    Scopes,
    GroupsClaim,
    AdminGroups,
    AllowedGroups,
    Enabled,
    Position,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}

fn text(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col).text().not_null().to_owned()
}

fn text_null(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col).text().null().to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AuthProviders::Table)
                    .col(text(AuthProviders::Id).primary_key())
                    .col(text(AuthProviders::Kind))
                    .col(text(AuthProviders::DisplayName))
                    .col(text(AuthProviders::Icon))
                    .col(text_null(AuthProviders::IssuerUrl))
                    .col(text_null(AuthProviders::Tenant))
                    .col(text(AuthProviders::ClientId))
                    .col(
                        ColumnDef::new(AuthProviders::ClientSecretEnc)
                            .blob()
                            .not_null(),
                    )
                    .col(text(AuthProviders::Scopes))
                    .col(text_null(AuthProviders::GroupsClaim))
                    // JSON arrays of strings.
                    .col(text(AuthProviders::AdminGroups))
                    .col(text(AuthProviders::AllowedGroups))
                    .col(ColumnDef::new(AuthProviders::Enabled).boolean().not_null())
                    .col(ColumnDef::new(AuthProviders::Position).integer().not_null())
                    .col(
                        ColumnDef::new(AuthProviders::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthProviders::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AuthProviders::DeletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
