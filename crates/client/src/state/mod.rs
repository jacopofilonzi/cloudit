//! Local engine state: one SQLite database (`<data dir>/state.db`, WAL) through
//! `rusqlite`.
//!
//! Migrations are the ordered SQL scripts of [`migrations::MIGRATIONS`]; the database's
//! `PRAGMA user_version` is the number applied. At open, each missing script runs in its
//! own transaction that also bumps `user_version`. Scripts are append-only (never edit
//! a released one); a database with a higher version than this build knows (a
//! downgrade) is refused.
//!
//! Tables (v1):
//!
//! - `account`: the one signed-in account (`id = 1`): server URL, user, device, where
//!   the token is stored (`keyring` | `file`) and when it expires. The token itself is
//!   never in this database.
//! - `folders`: linked folders: local path, server scope / folder ids and cached names,
//!   ignore patterns (one per line), paused flag, the **base** (last synced snapshot id
//!   and its stored manifest), the pending conflict snapshot this device uploaded (id and
//!   manifest), a cache of the last downloaded head manifest, and the last status.
//! - `file_index`: per folder, relative path → size, mtime, executable, chunk list: lets
//!   the scanner skip hashing files whose size and mtime didn't change. A cache only:
//!   deleting it costs a re-hash.
//! - `activity`: a bounded log of operations (newest [`activity::MAX_ROWS`] kept).
//! - `meta`: key/value pairs (`last_account`: the server + user the folder links belong to).

pub mod account;
pub mod activity;
pub mod folders;
pub mod index;
mod migrations;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::error::{Error, Result};

/// The engine's database handle. Statements are short; the mutex is never held across
/// an `.await`.
pub struct Db {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Db")
    }
}

impl Db {
    /// Opens (creating if needed) and migrates the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::State("the state database lock is poisoned".to_owned()))
    }

    /// Runs `f` on the connection.
    pub(crate) fn with<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let mut conn = self.lock()?;
        Ok(f(&mut conn)?)
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.lock()?;
        let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let known = i64::try_from(migrations::MIGRATIONS.len())
            .map_err(|_| Error::State("too many migrations".to_owned()))?;
        if current > known {
            return Err(Error::State(format!(
                "the state database has version {current}, this build knows {known}: \
                 it was written by a newer cloudit"
            )));
        }
        for (index, script) in migrations::MIGRATIONS.iter().enumerate().skip(
            usize::try_from(current).map_err(|_| Error::State("bad user_version".to_owned()))?,
        ) {
            let version =
                i64::try_from(index + 1).map_err(|_| Error::State("bad version".to_owned()))?;
            let tx = conn.transaction()?;
            tx.execute_batch(script)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
            tracing::info!(version, "migrated the local state database");
        }
        Ok(())
    }

    /// A `meta` value.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| {
            c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
                .optional()
        })
    }

    /// Sets a `meta` value.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(drop)
        })
    }
}

/// Now as RFC 3339 UTC with seconds (`2026-01-31T12:00:00Z`), like the API.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_once_and_refuses_newer_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let db = Db::open(&path).unwrap();
            db.set_meta("k", "v").unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.meta("k").unwrap().as_deref(), Some("v"));
        let version: i64 = db
            .with(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
            .unwrap();
        assert_eq!(
            version,
            i64::try_from(migrations::MIGRATIONS.len()).unwrap()
        );
        db.with(|c| c.pragma_update(None, "user_version", 999))
            .unwrap();
        drop(db);
        let err = Db::open(&path).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::State);
    }
}
