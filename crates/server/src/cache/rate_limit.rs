//! Fixed-window rate limiting, keyed by arbitrary strings (e.g. `login:<ip>`).
//!
//! Windows are aligned on the Unix clock (`window_index = now / window`), so every
//! instance counts in the same window. With Redis the counter is shared
//! (`cloudit:rl:<key>:<window_index>`, `SET NX PX` + `INCR` in one `MULTI`); without
//! Redis, or while it is failing, each instance counts in memory, so the effective limit
//! becomes per instance: acceptable for abuse protection, which is all this is for.
//!
//! Fixed windows allow up to twice the limit across a window boundary; limits are
//! chosen with that in mind.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Redis, now_ms};

/// Prefix of every rate-limit key in Redis.
const REDIS_PREFIX: &str = "cloudit:rl:";
/// The in-memory map is swept of stale windows when it grows past this many keys.
const MEMORY_SWEEP_THRESHOLD: usize = 10_000;

/// At most `max` hits per `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub max: u32,
    pub window: Duration,
}

impl RateLimit {
    #[must_use]
    pub const fn new(max: u32, window: Duration) -> Self {
        Self { max, window }
    }

    fn window_ms(self) -> u64 {
        u64::try_from(self.window.as_millis())
            .unwrap_or(u64::MAX)
            .max(1)
    }
}

/// Outcome of one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// Allowed; `remaining` more hits fit in this window.
    Allowed { remaining: u32 },
    /// Over the limit; retry after this long (end of the current window).
    Limited { retry_after: Duration },
}

impl RateDecision {
    #[must_use]
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    fn from_count(count: u64, limit: RateLimit, now_ms: u64) -> Self {
        if count <= u64::from(limit.max) {
            let remaining = u64::from(limit.max) - count;
            Self::Allowed {
                remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
            }
        } else {
            let window_ms = limit.window_ms();
            let left = window_ms - now_ms % window_ms;
            Self::Limited {
                retry_after: Duration::from_millis(left),
            }
        }
    }
}

/// Rate limiter shared by all handlers: Redis when available, memory otherwise.
#[derive(Clone, Debug, Default)]
pub struct RateLimiter {
    redis: Option<Redis>,
    memory: Arc<MemoryCounter>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(redis: Option<Redis>) -> Self {
        Self {
            redis,
            memory: Arc::default(),
        }
    }

    /// Counts one hit on `key` and says whether it is allowed.
    pub async fn hit(&self, key: &str, limit: RateLimit) -> RateDecision {
        let now = now_ms();
        let window = now / limit.window_ms();
        if let Some(redis) = &self.redis {
            let redis_key = format!("{REDIS_PREFIX}{key}:{window}");
            let window_ms = limit.window_ms();
            let count = redis
                .run("rate limit", |mut conn| async move {
                    let (count,): (u64,) = redis::pipe()
                        .atomic()
                        .cmd("SET")
                        .arg(&redis_key)
                        .arg(0)
                        .arg("PX")
                        .arg(window_ms)
                        .arg("NX")
                        .ignore()
                        .cmd("INCR")
                        .arg(&redis_key)
                        .query_async(&mut conn)
                        .await?;
                    Ok(count)
                })
                .await;
            if let Some(count) = count {
                return RateDecision::from_count(count, limit, now);
            }
        }
        self.memory.hit(key, limit, now)
    }
}

/// In-memory fixed-window counters: `key -> (window index, count)`.
#[derive(Debug, Default)]
struct MemoryCounter {
    windows: Mutex<HashMap<String, (u64, u64)>>,
}

impl MemoryCounter {
    fn hit(&self, key: &str, limit: RateLimit, now_ms: u64) -> RateDecision {
        let window_ms = limit.window_ms();
        let window = now_ms / window_ms;
        // A poisoned lock only means another thread panicked mid-update of a counter:
        // the map is still usable.
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if windows.len() >= MEMORY_SWEEP_THRESHOLD {
            // Entries from an older window than ours are dead for any limit at least as
            // long as this one; shorter-window keys get recreated on their next hit.
            windows.retain(|_, (w, _)| *w >= window);
        }
        let entry = windows.entry(key.to_owned()).or_insert((window, 0));
        if entry.0 != window {
            *entry = (window, 0);
        }
        entry.1 = entry.1.saturating_add(1);
        RateDecision::from_count(entry.1, limit, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: RateLimit = RateLimit::new(3, Duration::from_secs(60));

    #[test]
    fn memory_allows_up_to_the_limit_per_window() {
        let counter = MemoryCounter::default();
        let start = 60_000 * 1000; // aligned on a window boundary
        assert_eq!(
            counter.hit("k", LIMIT, start),
            RateDecision::Allowed { remaining: 2 }
        );
        assert_eq!(
            counter.hit("k", LIMIT, start + 1),
            RateDecision::Allowed { remaining: 1 }
        );
        assert_eq!(
            counter.hit("k", LIMIT, start + 2),
            RateDecision::Allowed { remaining: 0 }
        );
        assert_eq!(
            counter.hit("k", LIMIT, start + 10_000),
            RateDecision::Limited {
                retry_after: Duration::from_secs(50)
            }
        );
        // Other keys are independent.
        assert!(counter.hit("other", LIMIT, start + 10_000).is_allowed());
        // Next window: counting restarts.
        assert_eq!(
            counter.hit("k", LIMIT, start + 60_000),
            RateDecision::Allowed { remaining: 2 }
        );
    }

    #[test]
    fn memory_sweeps_stale_windows() {
        let counter = MemoryCounter::default();
        for i in 0..MEMORY_SWEEP_THRESHOLD {
            counter.hit(&format!("k{i}"), LIMIT, 0);
        }
        counter.hit("fresh", LIMIT, 120_000);
        let len = counter
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        assert_eq!(len, 1);
    }

    #[tokio::test]
    async fn without_redis_uses_memory() {
        let limiter = RateLimiter::new(None);
        let limit = RateLimit::new(2, Duration::from_secs(3600));
        let key = format!("test:{}", uuid::Uuid::new_v4());
        assert!(limiter.hit(&key, limit).await.is_allowed());
        assert!(limiter.hit(&key, limit).await.is_allowed());
        assert!(!limiter.hit(&key, limit).await.is_allowed());
    }

    #[tokio::test]
    async fn unreachable_redis_falls_back_to_memory() {
        let redis = Redis::connect(&crate::config::Secret::new("redis://127.0.0.1:1")).unwrap();
        let limiter = RateLimiter::new(Some(redis));
        let limit = RateLimit::new(1, Duration::from_secs(3600));
        assert!(limiter.hit("k", limit).await.is_allowed());
        assert!(!limiter.hit("k", limit).await.is_allowed());
    }

    #[tokio::test]
    async fn live_redis_is_shared_between_limiters() {
        let Some(redis) = crate::cache::tests::live_redis() else {
            eprintln!("skipped: set TEST_REDIS_URL (make test-ha) to run against Redis");
            return;
        };
        // Two limiters = two instances: the count is shared through Redis.
        let a = RateLimiter::new(Some(redis.clone()));
        let b = RateLimiter::new(Some(redis));
        let limit = RateLimit::new(2, Duration::from_secs(3600));
        let key = format!("test:{}", uuid::Uuid::new_v4());
        assert!(a.hit(&key, limit).await.is_allowed());
        assert!(b.hit(&key, limit).await.is_allowed());
        assert!(!a.hit(&key, limit).await.is_allowed());
        assert!(!b.hit(&key, limit).await.is_allowed());
    }
}
