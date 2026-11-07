//! Optional Redis (or Valkey) connection, from `REDIS_URL`.
//!
//! Redis is an accelerator, never a dependency: the server starts without it and no
//! request fails because of it. Callers go through [`Redis::run`], which returns `None`
//! on any error or timeout so they fall back (in-memory rate limit, polling…).
//!
//! - The connection is lazy (`ConnectionManager::new_lazy_with_config`): startup never
//!   waits for Redis, and the manager reconnects by itself.
//! - Every command has a short timeout. After a failure Redis is skipped for
//!   [`BACKOFF`] (a tiny circuit breaker), so an outage doesn't add the timeout to every
//!   request.
//! - Failures log a warning at most once per [`WARN_INTERVAL`], with the number of
//!   failures suppressed in between.
//!
//! Leases do **not** use Redis (see `db::lease`).

pub mod rate_limit;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::aio::{ConnectionManager, ConnectionManagerConfig};

use crate::config::Secret;

/// Upper bound for one Redis command, including a reconnect attempt.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
/// How long Redis is skipped after a failure.
const BACKOFF: Duration = Duration::from_secs(5);
/// Minimum interval between two "Redis unavailable" warnings.
const WARN_INTERVAL: Duration = Duration::from_secs(60);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[derive(Debug, Default)]
struct Health {
    /// Unix ms until which Redis is skipped (0 = available).
    down_until_ms: AtomicU64,
    /// Unix ms of the last warning logged.
    last_warn_ms: AtomicU64,
    /// Failures not logged since the last warning.
    suppressed: AtomicU64,
}

/// Shared Redis handle; cheap to clone.
#[derive(Clone)]
pub struct Redis {
    conn: ConnectionManager,
    health: Arc<Health>,
}

impl std::fmt::Debug for Redis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redis").finish_non_exhaustive()
    }
}

impl Redis {
    /// Prepares a lazy connection. Fails only if the URL is invalid; reachability is
    /// never checked here.
    pub fn connect(url: &Secret) -> anyhow::Result<Self> {
        // The URL may hold a password: the error must not include it.
        let client = redis::Client::open(url.expose())
            .map_err(|e| anyhow::anyhow!("invalid REDIS_URL ({:?})", e.kind()))?;
        let config = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(Duration::from_secs(1)))
            .set_response_timeout(Some(Duration::from_secs(1)))
            .set_number_of_retries(1)
            .set_max_delay(Duration::from_millis(500));
        let conn = ConnectionManager::new_lazy_with_config(client, config)
            .map_err(|e| anyhow::anyhow!("preparing the Redis connection: {e}"))?;
        Ok(Self {
            conn,
            health: Arc::new(Health::default()),
        })
    }

    /// Runs `f` with a connection unless Redis is in backoff. `None` on error or timeout
    /// (logged, rate-limited), so the caller degrades.
    pub async fn run<T, F, Fut>(&self, what: &'static str, f: F) -> Option<T>
    where
        F: FnOnce(ConnectionManager) -> Fut,
        Fut: Future<Output = redis::RedisResult<T>>,
    {
        if now_ms() < self.health.down_until_ms.load(Ordering::Relaxed) {
            return None;
        }
        self.run_now(what, f).await
    }

    async fn run_now<T, F, Fut>(&self, what: &'static str, f: F) -> Option<T>
    where
        F: FnOnce(ConnectionManager) -> Fut,
        Fut: Future<Output = redis::RedisResult<T>>,
    {
        match tokio::time::timeout(COMMAND_TIMEOUT, f(self.conn.clone())).await {
            Ok(Ok(value)) => {
                self.health.down_until_ms.store(0, Ordering::Relaxed);
                Some(value)
            }
            Ok(Err(err)) => {
                self.failed(what, &err);
                None
            }
            Err(_) => {
                self.failed(what, &"timed out");
                None
            }
        }
    }

    /// Readiness check: a `PING`, ignoring the backoff.
    pub async fn ping(&self) -> bool {
        self.run_now("ping", |mut conn| async move {
            redis::cmd("PING").query_async::<String>(&mut conn).await
        })
        .await
        .is_some()
    }

    fn failed(&self, what: &'static str, err: &dyn std::fmt::Display) {
        let now = now_ms();
        let backoff_ms = u64::try_from(BACKOFF.as_millis()).unwrap_or(u64::MAX);
        self.health
            .down_until_ms
            .store(now.saturating_add(backoff_ms), Ordering::Relaxed);

        let last = self.health.last_warn_ms.load(Ordering::Relaxed);
        let interval_ms = u64::try_from(WARN_INTERVAL.as_millis()).unwrap_or(u64::MAX);
        let due = now.saturating_sub(last) >= interval_ms;
        if due
            && self
                .health
                .last_warn_ms
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let suppressed = self.health.suppressed.swap(0, Ordering::Relaxed);
            tracing::warn!(
                operation = what,
                error = %err,
                suppressed,
                "Redis unavailable; degrading (next warning in at most {}s)",
                WARN_INTERVAL.as_secs()
            );
        } else {
            self.health.suppressed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_redis_degrades_quickly() {
        // Port 1 on localhost: connection refused, nothing listens there.
        let redis = Redis::connect(&Secret::new("redis://127.0.0.1:1")).unwrap();
        let started = std::time::Instant::now();
        assert!(!redis.ping().await);
        // In backoff now: skipped without trying.
        let skipped = redis
            .run("test", |mut conn| async move {
                redis::cmd("PING").query_async::<String>(&mut conn).await
            })
            .await;
        assert!(skipped.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn invalid_url_is_rejected_without_echoing_it() {
        let err = Redis::connect(&Secret::new("redis://:hunter2@[bad")).unwrap_err();
        assert!(!format!("{err:#}").contains("hunter2"), "{err:#}");
    }

    /// `TEST_REDIS_URL` (see `make test-ha`), or `None` to skip.
    pub fn live_redis() -> Option<Redis> {
        let url = std::env::var("TEST_REDIS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())?;
        Some(Redis::connect(&Secret::new(url)).unwrap())
    }

    #[tokio::test]
    async fn live_ping() {
        let Some(redis) = live_redis() else {
            eprintln!("skipped: set TEST_REDIS_URL (make test-ha) to run against Redis");
            return;
        };
        assert!(redis.ping().await);
    }
}
