//! Database layer: SeaORM over SQLite (single instance) or PostgreSQL (HA), chosen at
//! runtime by `DATABASE_URL`.
//!
//! - [`connect`] opens the pool. Unset `DATABASE_URL` means SQLite at
//!   `<DATA_DIR>/cloudit.db` (created with its directory). Every SQLite connection gets
//!   `journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL` and a busy timeout.
//! - [`migrate::run`] applies pending migrations at startup (backup first on SQLite,
//!   advisory lock on PostgreSQL).
//! - [`migrations`] holds the schema history: append-only, written with the portable
//!   SeaORM schema builder so the same code runs on both backends.
//! - [`lease`]: named, expiring locks for background jobs across instances.
//! - [`entities`]: SeaORM models of the tables.
//!
//! # Conventions for tables (both backends)
//!
//! - **Ids** are UUID v7 (`uuid::Uuid::now_v7()`, time-ordered), column type
//!   `ColumnDef::uuid()`: native `uuid` on PostgreSQL; on SQLite sqlx stores the 16 raw
//!   bytes as a BLOB (the declared type is `uuid_text`, but the value is a blob). Always
//!   bind `Uuid` values, never strings, so both backends compare correctly.
//! - **Timestamps** are UTC `chrono::DateTime<Utc>` in `timestamp_with_time_zone()`
//!   columns: `timestamptz` on PostgreSQL; RFC 3339 text with `+00:00` on SQLite, which
//!   orders correctly as text because every value is UTC. Times come from the
//!   application clock (`Utc::now()`), so instances must keep their clocks in sync (NTP).
//! - Names are snake_case, tables plural (`leases`, `users`).
//! - Every migration forces a transaction (`use_transaction() -> Some(true)`): SQLite
//!   supports transactional DDL too, so a failed migration leaves no half-applied schema.
//! - Secrets are stored as SHA-256 hex in `*_hash` columns (`auth::tokens`), passwords as
//!   argon2id PHC strings; the setup token is the one plaintext secret (`setup_state`).
//! - Booleans are `boolean()` (SQLite stores 0/1); nullable timestamps mark state
//!   changes (`used_at`, `revoked_at`, `disabled_at`) instead of status columns.
//! - A transaction that writes starts with a write: on SQLite a transaction that read
//!   first can't take the write lock while another connection writes, and fails with
//!   `SQLITE_BUSY` instead of waiting for the busy timeout.
//!
//! Entities are SeaORM models without relations; the queries live in the
//! modules that own the tables (`accounts`, `auth::sessions`, `devices`).

pub mod entities;
pub mod lease;
pub mod migrate;
pub mod migrations;
#[cfg(test)]
pub mod testing;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectOptions, ConnectionTrait as _, Database, DatabaseConnection, DbBackend};

use crate::config::Config;

pub use sea_orm::DatabaseConnection as Db;

/// Default pool size for SQLite: one writer at a time anyway, a few readers (WAL).
const SQLITE_DEFAULT_MAX_CONNECTIONS: u32 = 8;
/// Default pool size for PostgreSQL, per instance.
const POSTGRES_DEFAULT_MAX_CONNECTIONS: u32 = 10;
/// How long a SQLite statement waits for a competing writer before `SQLITE_BUSY`.
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens the database pool described by `config` (see the module docs).
pub async fn connect(config: &Config) -> anyhow::Result<DatabaseConnection> {
    let url = match &config.database_url {
        Some(url) => url.expose().to_owned(),
        None => {
            std::fs::create_dir_all(&config.data_dir)
                .with_context(|| format!("creating DATA_DIR {}", config.data_dir.display()))?;
            return connect_sqlite_file(
                &config.data_dir.join("cloudit.db"),
                config.db_max_connections,
            )
            .await;
        }
    };
    connect_url(&url, config.db_max_connections).await
}

/// Opens a SQLite database file, creating it if missing. The path is set on the sqlx
/// options directly, so any character is fine (no URL escaping).
pub async fn connect_sqlite_file(
    path: &Path,
    max_connections: Option<u32>,
) -> anyhow::Result<DatabaseConnection> {
    let options = ConnectOptions::new("sqlite://cloudit.db?mode=rwc");
    open(
        options,
        DbBackend::Sqlite,
        max_connections,
        Some(path.to_path_buf()),
    )
    .await
    .context("opening the SQLite database")
}

/// Opens `sqlite:…` or `postgres://…`. The URL may carry a password: it never appears
/// in errors or logs.
pub async fn connect_url(
    url: &str,
    max_connections: Option<u32>,
) -> anyhow::Result<DatabaseConnection> {
    let backend = if url.starts_with("sqlite:") {
        DbBackend::Sqlite
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        DbBackend::Postgres
    } else {
        anyhow::bail!("DATABASE_URL must start with sqlite: or postgres://");
    };
    // SeaORM echoes unparsable URLs in its error: check first, without the value.
    if url::Url::parse(url).is_err() {
        anyhow::bail!("DATABASE_URL is not a valid URL");
    }
    open(ConnectOptions::new(url), backend, max_connections, None)
        .await
        .with_context(|| format!("connecting to the {} database", backend_name(backend)))
}

async fn open(
    mut options: ConnectOptions,
    backend: DbBackend,
    max_connections: Option<u32>,
    sqlite_file: Option<PathBuf>,
) -> anyhow::Result<DatabaseConnection> {
    let default_max = match backend {
        DbBackend::Sqlite => SQLITE_DEFAULT_MAX_CONNECTIONS,
        _ => POSTGRES_DEFAULT_MAX_CONNECTIONS,
    };
    options
        .max_connections(max_connections.unwrap_or(default_max))
        .min_connections(1)
        .connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10))
        // Statements are logged by tracing spans at debug level, not by sqlx at info.
        .sqlx_logging(false);
    if backend == DbBackend::Sqlite {
        options.map_sqlx_sqlite_opts(move |o| {
            let o = match &sqlite_file {
                Some(path) => o.filename(path).create_if_missing(true),
                None => o,
            };
            o.journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(SQLITE_BUSY_TIMEOUT)
        });
    }
    let db = Database::connect(options).await?;
    db.execute_unprepared("SELECT 1")
        .await
        .context("first query")?;
    Ok(db)
}

/// Human-readable backend name for logs and errors.
#[must_use]
pub fn backend_name(backend: DbBackend) -> &'static str {
    match backend {
        DbBackend::Sqlite => "SQLite",
        DbBackend::Postgres => "PostgreSQL",
        _ => "unsupported",
    }
}

/// Readiness probe: one trivial round trip.
pub async fn ping(db: &DatabaseConnection) -> anyhow::Result<()> {
    db.execute_unprepared("SELECT 1")
        .await
        .context("SELECT 1")?;
    Ok(())
}
