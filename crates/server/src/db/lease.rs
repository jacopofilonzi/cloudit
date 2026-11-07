//! Leases: named, expiring locks in the `leases` table, so a background job runs on one
//! instance at a time (e.g. GC, retention) without any coordinator.
//!
//! A lease row is `(name, holder, expires_at)`. Every operation is a single atomic
//! statement whose affected-row count says whether it won; nothing is read first:
//!
//! - [`Leases::try_acquire`]: `INSERT … ON CONFLICT (name) DO UPDATE … WHERE
//!   holder = me OR expires_at < now` — takes a free or expired lease, or extends our
//!   own. Works on SQLite (3.24+) and PostgreSQL alike.
//! - [`Leases::renew`]: `UPDATE … WHERE name AND holder = me AND expires_at >= now`.
//!   Fails once the lease has expired, even if nobody took it: the holder must assume
//!   it lost the lease and stop.
//! - [`Leases::release`]: `DELETE … WHERE name AND holder = me`.
//!
//! The holder is the instance id (random per process start). Times come from the
//! instance clock, so clocks must be in sync (NTP) and TTLs much longer than any skew.
//!
//! Redis is deliberately **not** used for leases even when configured: one mechanism,
//! always available, transactional with the data it protects.

use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict, Query};
use sea_orm::{ConnectionTrait as _, DatabaseConnection, DeriveIden};

#[derive(DeriveIden)]
enum LeasesTable {
    #[sea_orm(iden = "leases")]
    Table,
    Name,
    Holder,
    ExpiresAt,
}

/// Lease operations for one holder (this instance).
#[derive(Clone)]
pub struct Leases {
    db: DatabaseConnection,
    holder: String,
}

impl Leases {
    #[must_use]
    pub fn new(db: DatabaseConnection, holder: impl Into<String>) -> Self {
        Self {
            db,
            holder: holder.into(),
        }
    }

    #[must_use]
    pub fn holder(&self) -> &str {
        &self.holder
    }

    /// Takes lease `name` for `ttl` if it is free, expired or already ours (then it is
    /// extended). Returns whether we hold it now.
    pub async fn try_acquire(&self, name: &str, ttl: Duration) -> anyhow::Result<bool> {
        self.try_acquire_at(name, ttl, Utc::now()).await
    }

    async fn try_acquire_at(
        &self,
        name: &str,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let expires_at = expiry(now, ttl)?;
        let stmt = Query::insert()
            .into_table(LeasesTable::Table)
            .columns([
                LeasesTable::Name,
                LeasesTable::Holder,
                LeasesTable::ExpiresAt,
            ])
            .values_panic([name.into(), self.holder.clone().into(), expires_at.into()])
            .on_conflict(
                OnConflict::column(LeasesTable::Name)
                    .update_columns([LeasesTable::Holder, LeasesTable::ExpiresAt])
                    .action_and_where(
                        Expr::col((LeasesTable::Table, LeasesTable::Holder))
                            .eq(self.holder.clone())
                            .or(Expr::col((LeasesTable::Table, LeasesTable::ExpiresAt)).lt(now)),
                    )
                    .to_owned(),
            )
            .to_owned();
        let result = self
            .db
            .execute(&stmt)
            .await
            .with_context(|| format!("acquiring lease {name}"))?;
        Ok(result.rows_affected() == 1)
    }

    /// Extends lease `name` by `ttl` from now if we still hold it and it hasn't expired.
    /// `false` means the lease is lost: stop the work it protects.
    pub async fn renew(&self, name: &str, ttl: Duration) -> anyhow::Result<bool> {
        self.renew_at(name, ttl, Utc::now()).await
    }

    async fn renew_at(
        &self,
        name: &str,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let expires_at = expiry(now, ttl)?;
        let stmt = Query::update()
            .table(LeasesTable::Table)
            .value(LeasesTable::ExpiresAt, expires_at)
            .and_where(Expr::col(LeasesTable::Name).eq(name))
            .and_where(Expr::col(LeasesTable::Holder).eq(self.holder.clone()))
            .and_where(Expr::col(LeasesTable::ExpiresAt).gte(now))
            .to_owned();
        let result = self
            .db
            .execute(&stmt)
            .await
            .with_context(|| format!("renewing lease {name}"))?;
        Ok(result.rows_affected() == 1)
    }

    /// Gives up lease `name` if we hold it (expired or not). Returns whether a row was
    /// removed.
    pub async fn release(&self, name: &str) -> anyhow::Result<bool> {
        let stmt = Query::delete()
            .from_table(LeasesTable::Table)
            .and_where(Expr::col(LeasesTable::Name).eq(name))
            .and_where(Expr::col(LeasesTable::Holder).eq(self.holder.clone()))
            .to_owned();
        let result = self
            .db
            .execute(&stmt)
            .await
            .with_context(|| format!("releasing lease {name}"))?;
        Ok(result.rows_affected() == 1)
    }
}

fn expiry(now: DateTime<Utc>, ttl: Duration) -> anyhow::Result<DateTime<Utc>> {
    let ttl = chrono::TimeDelta::from_std(ttl).context("lease TTL too large")?;
    now.checked_add_signed(ttl)
        .context("lease expiry overflows")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::TestDb;

    const TTL: Duration = Duration::from_secs(30);

    fn name() -> String {
        format!("job-{}", uuid::Uuid::now_v7().simple())
    }

    #[tokio::test]
    async fn acquire_renew_release() {
        let test = TestDb::migrated().await;
        let a = Leases::new(test.db.clone(), "instance-a");
        let job = name();

        assert!(a.try_acquire(&job, TTL).await.unwrap());
        // Re-acquiring our own lease extends it.
        assert!(a.try_acquire(&job, TTL).await.unwrap());
        assert!(a.renew(&job, TTL).await.unwrap());
        assert!(a.release(&job).await.unwrap());
        assert!(!a.release(&job).await.unwrap(), "already released");
        assert!(!a.renew(&job, TTL).await.unwrap(), "nothing to renew");
    }

    #[tokio::test]
    async fn two_holders_compete() {
        let test = TestDb::migrated().await;
        let other_db = test.second_connection().await;
        let a = Leases::new(test.db.clone(), "instance-a");
        let b = Leases::new(other_db, "instance-b");
        let job = name();

        assert!(a.try_acquire(&job, TTL).await.unwrap());
        assert!(!b.try_acquire(&job, TTL).await.unwrap());
        assert!(!b.renew(&job, TTL).await.unwrap());
        assert!(
            !b.release(&job).await.unwrap(),
            "b cannot release a's lease"
        );
        assert!(a.renew(&job, TTL).await.unwrap());

        assert!(a.release(&job).await.unwrap());
        assert!(b.try_acquire(&job, TTL).await.unwrap());
        assert!(!a.try_acquire(&job, TTL).await.unwrap());
    }

    #[tokio::test]
    async fn concurrent_acquire_has_one_winner() {
        let test = TestDb::migrated().await;
        let other_db = test.second_connection().await;
        let job = name();
        for round in 0..5 {
            let a = Leases::new(test.db.clone(), "instance-a");
            let b = Leases::new(other_db.clone(), "instance-b");
            let job = format!("{job}-{round}");
            let (ra, rb) = tokio::join!(a.try_acquire(&job, TTL), b.try_acquire(&job, TTL));
            let (ra, rb) = (ra.unwrap(), rb.unwrap());
            assert!(
                ra != rb,
                "round {round}: exactly one winner, got a={ra} b={rb}"
            );
        }
    }

    #[tokio::test]
    async fn expired_lease_is_taken_over() {
        let test = TestDb::migrated().await;
        let a = Leases::new(test.db.clone(), "instance-a");
        let b = Leases::new(test.db.clone(), "instance-b");
        let job = name();
        let now = Utc::now();

        assert!(a.try_acquire_at(&job, TTL, now).await.unwrap());
        // Still valid just before expiry.
        let almost = now + chrono::TimeDelta::seconds(29);
        assert!(!b.try_acquire_at(&job, TTL, almost).await.unwrap());
        // Expired: b takes it over, and a can no longer renew or re-take it.
        let later = now + chrono::TimeDelta::seconds(31);
        assert!(!a.renew_at(&job, TTL, later).await.unwrap(), "expired");
        assert!(b.try_acquire_at(&job, TTL, later).await.unwrap());
        assert!(!a.try_acquire_at(&job, TTL, later).await.unwrap());
        assert!(!a.release(&job).await.unwrap());
        assert!(b.renew_at(&job, TTL, later).await.unwrap());
    }
}
