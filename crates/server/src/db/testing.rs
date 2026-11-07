//! Test databases. With `TEST_DATABASE_URL` set (PostgreSQL, see `make test-ha`) each
//! test gets its own freshly created database on that server; otherwise a SQLite file
//! in a temporary directory. Never point `TEST_DATABASE_URL` at a real database: the
//! tests create databases named `cloudit_test_*` next to it.

use std::path::PathBuf;

use sea_orm::{ConnectionTrait as _, DatabaseConnection};
use tempfile::TempDir;

/// A database private to one test, plus a scratch directory.
pub struct TestDb {
    pub db: DatabaseConnection,
    pub dir: TempDir,
    /// PostgreSQL URL of this test's database; `None` for SQLite.
    pg_url: Option<String>,
}

/// `TEST_DATABASE_URL`, when set and non-empty.
pub fn test_database_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

impl TestDb {
    /// An empty database (no migrations applied).
    pub async fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        match test_database_url() {
            None => {
                let db = super::connect_sqlite_file(&dir.path().join("test.db"), Some(4))
                    .await
                    .unwrap();
                Self {
                    db,
                    dir,
                    pg_url: None,
                }
            }
            Some(admin_url) => {
                let name = format!("cloudit_test_{}", uuid::Uuid::now_v7().simple());
                let admin = super::connect_url(&admin_url, Some(1)).await.unwrap();
                admin
                    .execute_unprepared(&format!("CREATE DATABASE {name}"))
                    .await
                    .unwrap();
                admin.close().await.unwrap();
                let mut url = url::Url::parse(&admin_url).unwrap();
                url.set_path(&name);
                let url = url.to_string();
                let db = super::connect_url(&url, Some(4)).await.unwrap();
                Self {
                    db,
                    dir,
                    pg_url: Some(url),
                }
            }
        }
    }

    /// A database with every migration applied.
    pub async fn migrated() -> Self {
        let test = Self::empty().await;
        super::migrate::run(&test.db, &test.backups_dir())
            .await
            .unwrap();
        test
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.dir.path().join("backups")
    }

    /// Another pool on the same database, standing in for a second server instance.
    pub async fn second_connection(&self) -> DatabaseConnection {
        match &self.pg_url {
            Some(url) => super::connect_url(url, Some(4)).await.unwrap(),
            None => super::connect_sqlite_file(&self.dir.path().join("test.db"), Some(4))
                .await
                .unwrap(),
        }
    }
}
