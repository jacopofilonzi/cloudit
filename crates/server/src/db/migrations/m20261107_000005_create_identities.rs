//! Identity-provider sign-in: `identities` (an account's link to a provider subject),
//! `oauth_flows` (one started authorization, single use, 10 min) and `pending_signups`
//! (an authenticated but unknown identity waiting for onboarding, single use, 15 min).
//! Everything lives in the database so start, callback and onboarding can hit
//! different instances.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum AuthProviders {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Identities {
    Table,
    Id,
    UserId,
    ProviderId,
    Subject,
    Email,
    EmailVerified,
    CreatedAt,
    LastLoginAt,
}

#[derive(DeriveIden)]
enum PendingSignups {
    Table,
    Id,
    TokenHash,
    ProviderId,
    Subject,
    Email,
    EmailVerified,
    DisplayName,
    Groups,
    InviteTokenHash,
    CreatedAt,
    ExpiresAt,
    UsedAt,
}

#[derive(DeriveIden)]
enum OauthFlows {
    Table,
    Id,
    StateHash,
    BrowserBindingHash,
    ProviderId,
    Purpose,
    UserId,
    PendingSignupId,
    PkceVerifier,
    Nonce,
    ReturnTo,
    InviteTokenHash,
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

fn text(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col).text().not_null().to_owned()
}

fn text_null(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col).text().null().to_owned()
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
    to: (impl IntoIden, impl IntoIden),
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .name(name)
        .from(table, col)
        .to(to.0, to.1)
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Identities::Table)
                    .col(id(Identities::Id))
                    .col(ColumnDef::new(Identities::UserId).uuid().not_null())
                    .col(text(Identities::ProviderId))
                    .col(text(Identities::Subject))
                    .col(text_null(Identities::Email))
                    .col(
                        ColumnDef::new(Identities::EmailVerified)
                            .boolean()
                            .not_null(),
                    )
                    .col(timestamp(Identities::CreatedAt))
                    .col(timestamp_null(Identities::LastLoginAt))
                    .foreign_key(&mut fk(
                        "fk_identities_user",
                        Identities::Table,
                        Identities::UserId,
                        (Users::Table, Users::Id),
                    ))
                    .foreign_key(&mut fk(
                        "fk_identities_provider",
                        Identities::Table,
                        Identities::ProviderId,
                        (AuthProviders::Table, AuthProviders::Id),
                    ))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_identities_provider_subject")
                    .table(Identities::Table)
                    .col(Identities::ProviderId)
                    .col(Identities::Subject)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_identities_user_id")
                    .table(Identities::Table)
                    .col(Identities::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(PendingSignups::Table)
                    .col(id(PendingSignups::Id))
                    .col(text(PendingSignups::TokenHash).unique_key())
                    .col(text(PendingSignups::ProviderId))
                    .col(text(PendingSignups::Subject))
                    .col(text_null(PendingSignups::Email))
                    .col(
                        ColumnDef::new(PendingSignups::EmailVerified)
                            .boolean()
                            .not_null(),
                    )
                    .col(text(PendingSignups::DisplayName))
                    // JSON array of strings.
                    .col(text(PendingSignups::Groups))
                    .col(text_null(PendingSignups::InviteTokenHash))
                    .col(timestamp(PendingSignups::CreatedAt))
                    .col(timestamp(PendingSignups::ExpiresAt))
                    .col(timestamp_null(PendingSignups::UsedAt))
                    .foreign_key(&mut fk(
                        "fk_pending_signups_provider",
                        PendingSignups::Table,
                        PendingSignups::ProviderId,
                        (AuthProviders::Table, AuthProviders::Id),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(OauthFlows::Table)
                    .col(id(OauthFlows::Id))
                    .col(text(OauthFlows::StateHash).unique_key())
                    .col(text(OauthFlows::BrowserBindingHash))
                    .col(text(OauthFlows::ProviderId))
                    .col(text(OauthFlows::Purpose))
                    .col(ColumnDef::new(OauthFlows::UserId).uuid().null())
                    .col(ColumnDef::new(OauthFlows::PendingSignupId).uuid().null())
                    .col(text(OauthFlows::PkceVerifier))
                    .col(text(OauthFlows::Nonce))
                    .col(text(OauthFlows::ReturnTo))
                    .col(text_null(OauthFlows::InviteTokenHash))
                    .col(timestamp(OauthFlows::CreatedAt))
                    .col(timestamp(OauthFlows::ExpiresAt))
                    .col(timestamp_null(OauthFlows::UsedAt))
                    .foreign_key(&mut fk(
                        "fk_oauth_flows_provider",
                        OauthFlows::Table,
                        OauthFlows::ProviderId,
                        (AuthProviders::Table, AuthProviders::Id),
                    ))
                    .foreign_key(&mut fk(
                        "fk_oauth_flows_user",
                        OauthFlows::Table,
                        OauthFlows::UserId,
                        (Users::Table, Users::Id),
                    ))
                    .foreign_key(&mut fk(
                        "fk_oauth_flows_pending_signup",
                        OauthFlows::Table,
                        OauthFlows::PendingSignupId,
                        (PendingSignups::Table, PendingSignups::Id),
                    ))
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
