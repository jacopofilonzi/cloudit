//! Startup migration procedure.
//!
//! - **SQLite**: if migrations are pending and the database already has tables, it is
//!   first copied to `<DATA_DIR>/backups/cloudit-<UTC timestamp>-pre-migrate.db` with
//!   `VACUUM INTO` (a consistent snapshot even while the file is open). If the backup
//!   fails the server refuses to start. A brand-new database (no tables) is not backed
//!   up. Backups are never deleted automatically: the operator prunes them.
//! - **PostgreSQL**: migrations run while holding a session advisory lock with a fixed
//!   key, so when several instances start together exactly one migrates; the others
//!   wait, then find nothing pending. No backup is taken: back up with `pg_dump` before
//!   upgrading (README upgrade notes).
//!
//! The migrator refuses to run if the database has migrations this binary doesn't
//! know (a downgrade): restore the backup instead.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use sea_orm::sqlx;
use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait as _;

use super::migrations::Migrator;

/// `pg_advisory_lock` key guarding migrations: the bytes of `"cloudit\0"`.
const PG_MIGRATION_LOCK_KEY: i64 = 0x636c_6f75_6469_7400;

/// Applies pending migrations (see the module docs). `backups_dir` is used by SQLite only.
pub async fn run(db: &DatabaseConnection, backups_dir: &Path) -> anyhow::Result<()> {
    match db.get_database_backend() {
        DbBackend::Sqlite => run_sqlite(db, backups_dir).await,
        DbBackend::Postgres => run_postgres(db).await,
        other => anyhow::bail!("unsupported database backend {other:?}"),
    }
}

/// Lists pending migrations and applies them; returns how many were applied.
async fn apply_pending(db: &DatabaseConnection) -> anyhow::Result<usize> {
    let pending = Migrator::get_pending_migrations(db)
        .await
        .context("reading the migration status")?;
    if pending.is_empty() {
        return Ok(0);
    }
    let names: Vec<&str> = pending.iter().map(|m| m.name()).collect();
    tracing::info!(migrations = ?names, "applying database migrations");
    Migrator::up(db, None)
        .await
        .context("applying database migrations")?;
    Ok(pending.len())
}

async fn run_sqlite(db: &DatabaseConnection, backups_dir: &Path) -> anyhow::Result<()> {
    // Checked before the migrator creates its own bookkeeping table.
    let is_new = sqlite_table_count(db).await? == 0;
    let pending = Migrator::get_pending_migrations(db)
        .await
        .context("reading the migration status")?;
    if pending.is_empty() {
        tracing::info!("database schema is up to date");
        return Ok(());
    }
    if !is_new {
        let path = backup_sqlite(db, backups_dir)
            .await
            .context("backing up the SQLite database before migrating failed; refusing to start")?;
        tracing::info!(path = %path.display(), "database backed up before migrating");
    }
    let applied = apply_pending(db).await?;
    tracing::info!(applied, "database migrated");
    Ok(())
}

async fn sqlite_table_count(db: &DatabaseConnection) -> anyhow::Result<i64> {
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT count(*) AS n FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        ))
        .await
        .context("listing SQLite tables")?
        .context("listing SQLite tables: no row")?;
    row.try_get::<i64>("", "n")
        .context("listing SQLite tables: reading the count")
}

/// Writes a consistent copy of the open SQLite database into `dir`; returns its path.
pub async fn backup_sqlite(db: &DatabaseConnection, dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating the backup directory {}", dir.display()))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("cloudit-{stamp}-pre-migrate.db"));
    let target = path
        .to_str()
        .context("the backup path is not valid UTF-8")?
        .to_owned();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "VACUUM INTO ?",
        [target.into()],
    ))
    .await
    .with_context(|| format!("VACUUM INTO {}", path.display()))?;
    Ok(path)
}

async fn run_postgres(db: &DatabaseConnection) -> anyhow::Result<()> {
    let pool = db.get_postgres_connection_pool();
    // The lock belongs to this session: keep one dedicated connection for lock/unlock.
    let mut conn = pool
        .acquire()
        .await
        .context("acquiring a connection for the migration lock")?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(PG_MIGRATION_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await
        .context("taking the migration lock")?;
    if !acquired {
        tracing::info!("another instance is migrating the database; waiting for it");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(PG_MIGRATION_LOCK_KEY)
            .execute(&mut *conn)
            .await
            .context("waiting for the migration lock")?;
    }

    let result = apply_pending(db).await;

    let unlocked = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(PG_MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await;
    if let Err(err) = unlocked {
        // Closing the session releases the lock: don't return it to the pool.
        tracing::warn!(error = %err, "releasing the migration lock failed; closing the session");
        drop(conn.detach());
    }

    match result? {
        0 => tracing::info!("database schema is up to date"),
        applied => tracing::info!(applied, "database migrated"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::TestDb;

    async fn table_names(db: &DatabaseConnection) -> Vec<String> {
        let sql = match db.get_database_backend() {
            DbBackend::Sqlite => {
                "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
            }
            _ => {
                "SELECT table_name AS name FROM information_schema.tables \
                 WHERE table_schema = current_schema() ORDER BY table_name"
            }
        };
        db.query_all_raw(Statement::from_string(db.get_database_backend(), sql))
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get::<String>("", "name").unwrap())
            .collect()
    }

    #[tokio::test]
    async fn migrates_a_fresh_database_without_backup() {
        let test = TestDb::empty().await;
        run(&test.db, &test.backups_dir()).await.unwrap();

        let tables = table_names(&test.db).await;
        assert!(tables.contains(&"leases".to_owned()), "{tables:?}");
        assert!(
            Migrator::get_pending_migrations(&test.db)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !test.backups_dir().exists(),
            "a new database is not backed up"
        );

        // Idempotent: a second run finds nothing to do.
        run(&test.db, &test.backups_dir()).await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_backup_is_a_readable_copy() {
        let test = TestDb::empty().await;
        if test.db.get_database_backend() != DbBackend::Sqlite {
            return;
        }
        run(&test.db, &test.backups_dir()).await.unwrap();
        test.db
            .execute_unprepared(
                "INSERT INTO leases (name, holder, expires_at) \
                 VALUES ('job', 'a', '2030-01-01T00:00:00+00:00')",
            )
            .await
            .unwrap();

        let path = backup_sqlite(&test.db, &test.backups_dir()).await.unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("cloudit-") && name.ends_with("-pre-migrate.db"),
            "{name}"
        );

        let copy = crate::db::connect_sqlite_file(&path, Some(1))
            .await
            .unwrap();
        let row = copy
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT holder FROM leases WHERE name = 'job'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "holder").unwrap(), "a");
        copy.close().await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_refuses_to_migrate_when_backup_fails() {
        let test = TestDb::empty().await;
        if test.db.get_database_backend() != DbBackend::Sqlite {
            return;
        }
        // An existing table makes the database "not new", so a backup is required.
        test.db
            .execute_unprepared("CREATE TABLE legacy (id INTEGER)")
            .await
            .unwrap();
        // The backup dir is a file: creating it fails.
        let blocker = test.dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();

        let err = run(&test.db, &blocker).await.unwrap_err();
        assert!(format!("{err:#}").contains("refusing to start"), "{err:#}");
        let tables = table_names(&test.db).await;
        assert!(
            !tables.contains(&"leases".to_owned()),
            "nothing was migrated"
        );
    }

    #[tokio::test]
    async fn sqlite_backs_up_an_existing_database_before_migrating() {
        let test = TestDb::empty().await;
        if test.db.get_database_backend() != DbBackend::Sqlite {
            return;
        }
        test.db
            .execute_unprepared("CREATE TABLE legacy (id INTEGER)")
            .await
            .unwrap();
        run(&test.db, &test.backups_dir()).await.unwrap();
        let backups: Vec<_> = std::fs::read_dir(test.backups_dir()).unwrap().collect();
        assert_eq!(backups.len(), 1);
    }

    #[tokio::test]
    async fn postgres_concurrent_instances_migrate_once() {
        // SQLite is single-instance: there is nothing to coordinate.
        let test = TestDb::empty().await;
        if test.db.get_database_backend() != DbBackend::Postgres {
            return;
        }
        let other = test.second_connection().await;
        let backups = test.backups_dir();
        let (a, b) = tokio::join!(run(&test.db, &backups), run(&other, &backups));
        a.unwrap();
        b.unwrap();
        assert!(
            Migrator::get_pending_migrations(&test.db)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
