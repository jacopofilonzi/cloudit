//! `users.quota_unlimited`: an admin can exempt one account from the server's default
//! quota (`sync::usage::effective_quota`). NOT NULL, default false: existing accounts
//! keep their own quota or the server default, as before.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    QuotaUnlimited,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(
                        ColumnDef::new(Users::QuotaUnlimited)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
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

    use crate::db::entities::users;
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
        ApprovalStatus,
        SignupSource,
        CreatedAt,
        UpdatedAt,
        QuotaBytes,
    }

    /// Accounts from before this migration (one with its own quota, one on the server
    /// default) keep their quota and are not unlimited.
    #[tokio::test]
    async fn existing_users_are_not_unlimited() {
        let test = TestDb::empty().await;
        Migrator::up(&test.db, Some(10)).await.unwrap();

        let now = crate::time::now();
        let alice = Uuid::now_v7();
        let bob = Uuid::now_v7();
        for (id, email, quota) in [
            (alice, "alice@example.com", Some(5_000_000_000_i64)),
            (bob, "bob@example.com", None),
        ] {
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
                    Users::ApprovalStatus,
                    Users::SignupSource,
                    Users::CreatedAt,
                    Users::UpdatedAt,
                    Users::QuotaBytes,
                ])
                .values_panic([
                    id.into(),
                    email.into(),
                    true.into(),
                    "Someone".into(),
                    Some("$argon2id$v=19$x".to_owned()).into(),
                    (id == alice).into(),
                    false.into(),
                    "approved".into(),
                    Some("invite".to_owned()).into(),
                    now.into(),
                    now.into(),
                    quota.into(),
                ])
                .to_owned();
            test.db.execute(&stmt).await.unwrap();
        }

        Migrator::up(&test.db, None).await.unwrap();

        let all = users::Entity::find().all(&test.db).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|u| !u.quota_unlimited));
        let a = all.iter().find(|u| u.id == alice).unwrap();
        assert_eq!(a.quota_bytes, Some(5_000_000_000));
        let b = all.iter().find(|u| u.id == bob).unwrap();
        assert_eq!(b.quota_bytes, None);
        assert_eq!(b.email, "bob@example.com");
    }
}
