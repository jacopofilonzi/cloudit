//! Real-time events: the [`Events`] publisher and the per-instance fan-out to the SSE
//! streams of `GET /api/v1/events` (`routes::events`). Event types and the client
//! contract: `cloudit_core::protocol::events`.
//!
//! - **Recipients** are computed when publishing ([`notify`]): the scope's owner and
//!   members at that moment, plus the affected user of a membership change. A stream
//!   only forwards deliveries addressed to its user, so scope ids never reach
//!   non-members.
//! - **Publish after commit**: handlers and jobs publish once their database
//!   transaction has committed; an event never announces a change that could still roll
//!   back. Publishing never fails a request (errors are logged).
//! - **Single process**: a `tokio::sync::broadcast` channel; every stream subscribes and
//!   filters by user. A stream that falls behind (lagged) is closed: the client
//!   reconnects and resyncs with `/sync/heads`.
//! - **With Redis** ([`fanout`]): every event is delivered locally right away **and**
//!   queued for `PUBLISH cloudit:events` (one background task, so requests never wait on
//!   Redis and order is kept) as `{origin, recipients, event}`. Each instance holds one
//!   dedicated subscription to that channel (reconnecting with backoff, pinged every
//!   15 s) and delivers the messages of other origins to its local streams. Redis
//!   failures only degrade: delivery stays local, and [`Events::realtime`] reports
//!   `local` until the subscription is up and Redis is out of backoff again; streams
//!   send a fresh `hello` when that mode changes.
//! - **Limits**: at most [`MAX_STREAMS_PER_USER`] open streams per user and instance
//!   (more → `rate_limited`). The counter is in memory: it only protects this instance.

mod fanout;
pub mod notify;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cloudit_core::protocol::{RealtimeMode, ServerEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::cache::Redis;
use crate::errors::ApiError;

/// The Redis pub/sub channel shared by every instance.
pub const CHANNEL: &str = "cloudit:events";
/// Interval of the SSE keep-alive comments (and of the credential re-checks).
pub const KEEPALIVE: Duration = Duration::from_secs(25);
/// Open streams per user on one instance.
pub const MAX_STREAMS_PER_USER: usize = 20;
/// Deliveries a stream may lag behind before it is closed.
const LOCAL_CAPACITY: usize = 1024;

/// One event and who may receive it.
#[derive(Debug)]
pub struct Delivery {
    /// Sorted, deduplicated.
    recipients: Vec<Uuid>,
    pub event: ServerEvent,
}

impl Delivery {
    #[must_use]
    pub fn new(recipients: impl IntoIterator<Item = Uuid>, event: ServerEvent) -> Self {
        let mut recipients: Vec<Uuid> = recipients.into_iter().collect();
        recipients.sort_unstable();
        recipients.dedup();
        Self { recipients, event }
    }

    #[must_use]
    pub fn is_for(&self, user: Uuid) -> bool {
        self.recipients.binary_search(&user).is_ok()
    }
}

/// What travels through Redis.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    /// The publishing instance (it already delivered locally).
    origin: String,
    recipients: Vec<String>,
    event: ServerEvent,
}

struct Inner {
    instance_id: Arc<str>,
    local: broadcast::Sender<Arc<Delivery>>,
    redis: Option<Redis>,
    /// Payloads waiting for `PUBLISH` (`None` without Redis).
    outbox: Option<mpsc::Sender<String>>,
    /// The Redis subscription is up.
    subscribed: AtomicBool,
    streams: Mutex<HashMap<Uuid, usize>>,
    keepalive_ms: AtomicU64,
}

/// The event publisher and local fan-out; cheap to clone.
#[derive(Clone)]
pub struct Events {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events").finish_non_exhaustive()
    }
}

impl Events {
    /// Local fan-out, plus the Redis publisher and subscription tasks when `redis` is
    /// set (they stop when the last clone of this handle is dropped).
    #[must_use]
    pub fn new(instance_id: Arc<str>, redis: Option<Redis>) -> Self {
        let (local, _) = broadcast::channel(LOCAL_CAPACITY);
        let mut inner = Inner {
            instance_id,
            local,
            redis: redis.clone(),
            outbox: None,
            subscribed: AtomicBool::new(false),
            streams: Mutex::new(HashMap::new()),
            keepalive_ms: AtomicU64::new(millis(KEEPALIVE)),
        };
        let outbox = redis.map(fanout::start_publisher);
        inner.outbox = outbox;
        let inner = Arc::new(inner);
        if let Some(redis) = &inner.redis {
            fanout::start_subscriber(Arc::downgrade(&inner), redis.clone());
        }
        Self { inner }
    }

    /// Delivers `event` to the streams of `recipients`, on this instance now and on the
    /// others through Redis (if configured). Never fails.
    pub fn publish(&self, recipients: impl IntoIterator<Item = Uuid>, event: ServerEvent) {
        let delivery = Delivery::new(recipients, event);
        if delivery.recipients.is_empty() {
            return;
        }
        if let Some(outbox) = &self.inner.outbox {
            let envelope = Envelope {
                origin: self.inner.instance_id.to_string(),
                recipients: delivery.recipients.iter().map(Uuid::to_string).collect(),
                event: delivery.event.clone(),
            };
            match serde_json::to_string(&envelope) {
                Ok(payload) => {
                    if outbox.try_send(payload).is_err() {
                        tracing::warn!("event outbox full: an event reaches this instance only");
                    }
                }
                Err(err) => tracing::warn!(error = %err, "encoding an event"),
            }
        }
        self.deliver_local(delivery);
    }

    fn deliver_local(&self, delivery: Delivery) {
        // No receivers is fine: nobody is connected.
        let _ = self.inner.local.send(Arc::new(delivery));
    }

    /// A receiver of every delivery on this instance (streams filter by user).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Delivery>> {
        self.inner.local.subscribe()
    }

    /// `shared` while events cross instances (Redis subscription up and Redis not in
    /// backoff), else `local`.
    #[must_use]
    pub fn realtime(&self) -> RealtimeMode {
        match &self.inner.redis {
            Some(redis)
                if self.inner.subscribed.load(Ordering::Relaxed) && redis.is_available() =>
            {
                RealtimeMode::Shared
            }
            _ => RealtimeMode::Local,
        }
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.inner.instance_id
    }

    /// Interval of keep-alives and credential re-checks.
    #[must_use]
    pub fn keepalive(&self) -> Duration {
        Duration::from_millis(self.inner.keepalive_ms.load(Ordering::Relaxed))
    }

    /// Tests: a shorter keep-alive.
    #[cfg(test)]
    pub fn set_keepalive(&self, every: Duration) {
        self.inner
            .keepalive_ms
            .store(millis(every), Ordering::Relaxed);
    }

    /// Reserves one of the user's stream slots (released on drop); `rate_limited` when
    /// all [`MAX_STREAMS_PER_USER`] are taken on this instance.
    pub fn open_stream(&self, user: Uuid) -> Result<StreamSlot, ApiError> {
        let mut streams = self
            .inner
            .streams
            .lock()
            .map_err(|_| anyhow::anyhow!("stream counter poisoned"))?;
        let open = streams.entry(user).or_default();
        if *open >= MAX_STREAMS_PER_USER {
            return Err(ApiError::RateLimited {
                retry_after: Duration::from_secs(30),
            });
        }
        *open += 1;
        Ok(StreamSlot {
            events: self.clone(),
            user,
        })
    }
}

/// One open stream of a user (see [`Events::open_stream`]).
pub struct StreamSlot {
    events: Events,
    user: Uuid,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        if let Ok(mut streams) = self.events.inner.streams.lock()
            && let Some(open) = streams.get_mut(&self.user)
        {
            *open = open.saturating_sub(1);
            if *open == 0 {
                streams.remove(&self.user);
            }
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudit_core::protocol::{MembershipChangedEvent, UsageChangedEvent};

    fn event() -> ServerEvent {
        ServerEvent::MembershipChanged(MembershipChangedEvent {
            scope_id: "s".to_owned(),
        })
    }

    #[tokio::test]
    async fn local_delivery_is_filtered_by_recipient() {
        let events = Events::new("me".into(), None);
        assert_eq!(events.realtime(), RealtimeMode::Local);
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut rx = events.subscribe();
        events.publish([a, a], event());
        events.publish([], ServerEvent::UsageChanged(UsageChangedEvent {}));
        let got = rx.recv().await.unwrap();
        assert!(got.is_for(a));
        assert!(!got.is_for(b));
        assert!(rx.try_recv().is_err(), "nothing for no recipients");
    }

    #[test]
    fn stream_slots_are_limited_and_released() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = rt.enter();
        let events = Events::new("me".into(), None);
        let user = Uuid::now_v7();
        let slots: Vec<_> = (0..MAX_STREAMS_PER_USER)
            .map(|_| events.open_stream(user).unwrap())
            .collect();
        assert!(matches!(
            events.open_stream(user),
            Err(ApiError::RateLimited { .. })
        ));
        assert!(events.open_stream(Uuid::now_v7()).is_ok(), "per user");
        drop(slots);
        assert!(events.open_stream(user).is_ok());
    }
}
