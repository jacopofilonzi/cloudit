//! Background jobs, the garbage collector's pending object deletions and permanent
//! deletion requests from the trash.
//!
//! - `jobs`: one row per job name (inserted when missing at startup, see `jobs::store`):
//!   `requested_at` (an admin asked for an immediate run; any instance's scheduler
//!   notices it) and `last_started_at` (shared, so the instances don't each run a job
//!   at every interval).
//! - `job_runs`: one row per run (instance, start, end, outcome, counts as a JSON
//!   object, error), for the admin view; old finished runs are deleted by `cleanup`.
//! - `object_deletions`: objects whose row the garbage collector deleted and that it
//!   still has to delete from storage (the "tombstones" of the protocol in
//!   `sync::objects`). No foreign key: a scope purge deletes them with the scope.
//! - `scopes.purge_requested_at`, `folders.purge_requested_at`: set by the permanent
//!   delete endpoints on trashed items; the `trash_purge` job then deletes them without
//!   waiting for the retention period. NULL on existing rows.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Jobs {
    Table,
    Name,
    RequestedAt,
    LastStartedAt,
}

#[derive(DeriveIden)]
enum JobRuns {
    Table,
    Id,
    Job,
    InstanceId,
    StartedAt,
    FinishedAt,
    Outcome,
    Counts,
    Error,
}

#[derive(DeriveIden)]
enum ObjectDeletions {
    Table,
    ScopeId,
    Kind,
    Hash,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Scopes {
    Table,
    PurgeRequestedAt,
}

#[derive(DeriveIden)]
enum Folders {
    Table,
    PurgeRequestedAt,
}

fn timestamp(col: impl IntoIden) -> ColumnDef {
    ColumnDef::new(col)
        .timestamp_with_time_zone()
        .not_null()
        .to_owned()
}

fn nullable_timestamp(col: impl IntoIden) -> ColumnDef {
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
                    .table(Jobs::Table)
                    .col(ColumnDef::new(Jobs::Name).text().not_null().primary_key())
                    .col(nullable_timestamp(Jobs::RequestedAt))
                    .col(nullable_timestamp(Jobs::LastStartedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(JobRuns::Table)
                    .col(ColumnDef::new(JobRuns::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(JobRuns::Job).text().not_null())
                    .col(ColumnDef::new(JobRuns::InstanceId).text().not_null())
                    .col(timestamp(JobRuns::StartedAt))
                    .col(nullable_timestamp(JobRuns::FinishedAt))
                    .col(ColumnDef::new(JobRuns::Outcome).text().not_null())
                    .col(ColumnDef::new(JobRuns::Counts).text().not_null())
                    .col(ColumnDef::new(JobRuns::Error).text().null())
                    .to_owned(),
            )
            .await?;
        // Latest runs of a job (admin view; ids are UUID v7, time-ordered).
        manager
            .create_index(
                Index::create()
                    .name("idx_job_runs_job_id")
                    .table(JobRuns::Table)
                    .col(JobRuns::Job)
                    .col(JobRuns::Id)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ObjectDeletions::Table)
                    .col(ColumnDef::new(ObjectDeletions::ScopeId).uuid().not_null())
                    .col(ColumnDef::new(ObjectDeletions::Kind).text().not_null())
                    .col(ColumnDef::new(ObjectDeletions::Hash).text().not_null())
                    .col(timestamp(ObjectDeletions::CreatedAt))
                    .primary_key(
                        Index::create()
                            .col(ObjectDeletions::ScopeId)
                            .col(ObjectDeletions::Kind)
                            .col(ObjectDeletions::Hash),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Scopes::Table)
                    .add_column(nullable_timestamp(Scopes::PurgeRequestedAt))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Folders::Table)
                    .add_column(nullable_timestamp(Folders::PurgeRequestedAt))
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

    use crate::db::entities::{folders, scopes};
    use crate::db::migrations::Migrator;
    use crate::db::testing::TestDb;

    #[derive(sea_orm::DeriveIden)]
    enum Users {
        Table,
        Id,
        Email,
        EmailVerified,
        DisplayName,
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
        StoredBytes,
    }

    #[derive(sea_orm::DeriveIden)]
    enum Folders {
        Table,
        Id,
        ScopeId,
        Name,
        NameKey,
        ActiveKey,
        CreatedBy,
        CreatedAt,
        UpdatedAt,
        DeletedAt,
    }

    /// Scopes and folders from before this migration (some in the trash) keep their
    /// state and get no purge request.
    #[tokio::test]
    async fn existing_rows_are_not_marked_for_purge() {
        let test = TestDb::empty().await;
        Migrator::up(&test.db, Some(8)).await.unwrap();
        let now = crate::time::now();
        let user = Uuid::now_v7();
        test.db
            .execute(
                &Query::insert()
                    .into_table(Users::Table)
                    .columns([
                        Users::Id,
                        Users::Email,
                        Users::EmailVerified,
                        Users::DisplayName,
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
                        true.into(),
                        false.into(),
                        now.into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await
            .unwrap();
        let scope = Uuid::now_v7();
        test.db
            .execute(
                &Query::insert()
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
                        Scopes::StoredBytes,
                    ])
                    .values_panic([
                        scope.into(),
                        user.into(),
                        "Games".into(),
                        "games".into(),
                        None::<String>.into(),
                        now.into(),
                        now.into(),
                        Some(now).into(),
                        42i64.into(),
                    ])
                    .to_owned(),
            )
            .await
            .unwrap();
        for (name, deleted_at) in [("Saves", None), ("Old", Some(now))] {
            test.db
                .execute(
                    &Query::insert()
                        .into_table(Folders::Table)
                        .columns([
                            Folders::Id,
                            Folders::ScopeId,
                            Folders::Name,
                            Folders::NameKey,
                            Folders::ActiveKey,
                            Folders::CreatedBy,
                            Folders::CreatedAt,
                            Folders::UpdatedAt,
                            Folders::DeletedAt,
                        ])
                        .values_panic([
                            Uuid::now_v7().into(),
                            scope.into(),
                            name.into(),
                            name.to_lowercase().into(),
                            deleted_at.is_none().then(|| name.to_lowercase()).into(),
                            user.into(),
                            now.into(),
                            now.into(),
                            deleted_at.into(),
                        ])
                        .to_owned(),
                )
                .await
                .unwrap();
        }

        Migrator::up(&test.db, None).await.unwrap();

        let scope = scopes::Entity::find_by_id(scope)
            .one(&test.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scope.purge_requested_at, None);
        assert_eq!(scope.deleted_at, Some(now));
        assert_eq!(scope.stored_bytes, 42);
        let all = folders::Entity::find().all(&test.db).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|f| f.purge_requested_at.is_none()));
        assert_eq!(all.iter().filter(|f| f.deleted_at.is_some()).count(), 1);
    }
}
