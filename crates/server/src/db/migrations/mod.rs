//! Schema history. **Append-only**: never edit, reorder or remove a migration once it
//! has been released (pushed to `main`); change the schema by appending a new one.
//!
//! Rules for new migrations:
//! - file `mYYYYMMDD_NNNNNN_<what>.rs`, registered at the end of [`Migrator::migrations`];
//! - written with the SeaORM schema builder (`Table::create()`, `ColumnDef`…), never raw
//!   SQL for one backend, so the same code runs on SQLite and PostgreSQL; follow the
//!   table conventions in the [`crate::db`] docs (UUID v7 ids, UTC timestamps);
//! - `use_transaction()` returns `Some(true)`;
//! - a migration that touches existing rows gets a test that builds a database at the
//!   previous version with realistic data and migrates it;
//! - run `make test-ha` (PostgreSQL) besides `make test` (SQLite).
//!
//! `down` is not implemented: rollback is restoring the pre-migration backup.

mod m20261107_000001_create_leases;
mod m20261107_000002_create_accounts;
mod m20261107_000003_create_tokens;
mod m20261107_000004_create_auth_providers;
mod m20261107_000005_create_identities;
mod m20261107_000006_account_approval;
mod m20261107_000007_create_scopes;
mod m20261107_000008_create_sync;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

/// All migrations, in order.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261107_000001_create_leases::Migration),
            Box::new(m20261107_000002_create_accounts::Migration),
            Box::new(m20261107_000003_create_tokens::Migration),
            Box::new(m20261107_000004_create_auth_providers::Migration),
            Box::new(m20261107_000005_create_identities::Migration),
            Box::new(m20261107_000006_account_approval::Migration),
            Box::new(m20261107_000007_create_scopes::Migration),
            Box::new(m20261107_000008_create_sync::Migration),
        ]
    }
}
