//! Admin approval of new accounts.
//!
//! - `users.approval_status` (`approved` | `pending` | `rejected`, NOT NULL, default
//!   `approved`: every existing account stays able to sign in),
//!   `approval_decided_at` / `approval_decided_by` (the admin's last decision; null
//!   for automatic approvals) and `signup_source` (`password`, `invite` or the
//!   provider id; null for the first admin and for existing accounts).
//! - `auth_providers.new_accounts` (`auto` | `approval`, NOT NULL, default
//!   `approval`): existing providers get `approval` too.
//!
//! `approval_decided_by` has no foreign-key constraint: SQLite can't add one to an
//! existing table without rebuilding it. Users are never deleted, so it can't dangle.
//! SQLite also takes one change per `ALTER TABLE`, hence one statement per column.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    ApprovalStatus,
    ApprovalDecidedAt,
    ApprovalDecidedBy,
    SignupSource,
}

#[derive(DeriveIden)]
enum AuthProviders {
    Table,
    NewAccounts,
}

fn add(table: impl IntoIden, col: &mut ColumnDef) -> TableAlterStatement {
    Table::alter().table(table).add_column(col).to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(add(
                Users::Table,
                ColumnDef::new(Users::ApprovalStatus)
                    .text()
                    .not_null()
                    .default("approved"),
            ))
            .await?;
        manager
            .alter_table(add(
                Users::Table,
                ColumnDef::new(Users::ApprovalDecidedAt)
                    .timestamp_with_time_zone()
                    .null(),
            ))
            .await?;
        manager
            .alter_table(add(
                Users::Table,
                ColumnDef::new(Users::ApprovalDecidedBy).uuid().null(),
            ))
            .await?;
        manager
            .alter_table(add(
                Users::Table,
                ColumnDef::new(Users::SignupSource).text().null(),
            ))
            .await?;
        manager
            .alter_table(add(
                AuthProviders::Table,
                ColumnDef::new(AuthProviders::NewAccounts)
                    .text()
                    .not_null()
                    .default("approval"),
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

    use crate::db::entities::{auth_providers, users};
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
        DisabledAt,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(sea_orm::DeriveIden)]
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

    /// Accounts and providers as the previous version stored them, then this
    /// migration: every account stays approved, every provider requires approval.
    #[tokio::test]
    async fn existing_rows_get_the_defaults() {
        let test = TestDb::empty().await;
        Migrator::up(&test.db, Some(5)).await.unwrap();

        let now = crate::time::now();
        let admin = Uuid::now_v7();
        let disabled = Uuid::now_v7();
        let provider_only = Uuid::now_v7();
        let rows = [
            (
                admin,
                "admin@example.com",
                Some("$argon2id$v=19$x"),
                true,
                None,
            ),
            (
                disabled,
                "gone@example.com",
                Some("$argon2id$v=19$y"),
                false,
                Some(now),
            ),
            (provider_only, "sso@example.com", None, false, None),
        ];
        for (id, email, hash, is_admin, disabled_at) in rows {
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
                    Users::DisabledAt,
                    Users::CreatedAt,
                    Users::UpdatedAt,
                ])
                .values_panic([
                    id.into(),
                    email.into(),
                    true.into(),
                    email.into(),
                    hash.map(str::to_owned).into(),
                    is_admin.into(),
                    false.into(),
                    disabled_at.into(),
                    now.into(),
                    now.into(),
                ])
                .to_owned();
            test.db.execute(&stmt).await.unwrap();
        }
        for (id, kind, deleted_at) in [("sso", "keycloak", None), ("old", "github", Some(now))] {
            let stmt = Query::insert()
                .into_table(AuthProviders::Table)
                .columns([
                    AuthProviders::Id,
                    AuthProviders::Kind,
                    AuthProviders::DisplayName,
                    AuthProviders::Icon,
                    AuthProviders::IssuerUrl,
                    AuthProviders::Tenant,
                    AuthProviders::ClientId,
                    AuthProviders::ClientSecretEnc,
                    AuthProviders::Scopes,
                    AuthProviders::GroupsClaim,
                    AuthProviders::AdminGroups,
                    AuthProviders::AllowedGroups,
                    AuthProviders::Enabled,
                    AuthProviders::Position,
                    AuthProviders::CreatedAt,
                    AuthProviders::UpdatedAt,
                    AuthProviders::DeletedAt,
                ])
                .values_panic([
                    id.into(),
                    kind.into(),
                    "Company SSO".into(),
                    "generic".into(),
                    Some("https://idp.example.com/realms/main".to_owned()).into(),
                    None::<String>.into(),
                    "client".into(),
                    vec![1u8, 2, 3].into(),
                    "openid email profile".into(),
                    None::<String>.into(),
                    "[\"admins\"]".into(),
                    "[]".into(),
                    deleted_at.is_none().into(),
                    0i32.into(),
                    now.into(),
                    now.into(),
                    deleted_at.into(),
                ])
                .to_owned();
            test.db.execute(&stmt).await.unwrap();
        }

        Migrator::up(&test.db, None).await.unwrap();

        let all = users::Entity::find().all(&test.db).await.unwrap();
        assert_eq!(all.len(), 3);
        for user in &all {
            assert_eq!(user.approval_status, "approved", "{}", user.email);
            assert!(user.approval_decided_at.is_none());
            assert!(user.approval_decided_by.is_none());
            assert!(user.signup_source.is_none());
        }
        let gone = all.iter().find(|u| u.id == disabled).unwrap();
        assert!(gone.is_disabled() && !gone.is_active(), "still disabled");
        assert!(all.iter().find(|u| u.id == admin).unwrap().is_active());
        let providers = auth_providers::Entity::find().all(&test.db).await.unwrap();
        assert_eq!(providers.len(), 2);
        for provider in providers {
            assert_eq!(provider.new_accounts, "approval", "{}", provider.id);
            assert_eq!(provider.client_secret_enc, [1, 2, 3], "untouched");
        }
    }
}
