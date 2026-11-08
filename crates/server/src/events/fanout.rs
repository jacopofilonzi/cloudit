//! Events across instances through Redis pub/sub (see the [`super`] docs).
//!
//! - The **publisher** task drains the outbox with `PUBLISH` through the shared
//!   connection manager ([`Redis::run`]: timeout and backoff). A failed publish is
//!   dropped: the event was already delivered locally, and [`super::Events::realtime`]
//!   turns `local` while Redis is in backoff.
//! - The **subscriber** task holds one dedicated pub/sub connection, `SUBSCRIBE`s to
//!   [`CHANNEL`], pings it every [`PING_EVERY`] (a silently dead connection is noticed)
//!   and reconnects with exponential backoff (1 s to 30 s). It holds only a weak
//!   reference to the publisher state and stops when that is gone.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::StreamExt as _;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{CHANNEL, Delivery, Envelope, Inner};
use crate::cache::Redis;

const OUTBOX_CAPACITY: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PING_EVERY: Duration = Duration::from_secs(15);
const PING_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub(super) fn start_publisher(redis: Redis) -> mpsc::Sender<String> {
    let (tx, mut rx) = mpsc::channel::<String>(OUTBOX_CAPACITY);
    tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            redis
                .run("publish", |mut conn| async move {
                    redis::cmd("PUBLISH")
                        .arg(CHANNEL)
                        .arg(payload)
                        .query_async::<i64>(&mut conn)
                        .await
                })
                .await;
        }
    });
    tx
}

pub(super) fn start_subscriber(inner: Weak<Inner>, redis: Redis) {
    tokio::spawn(subscriber(inner, redis));
}

async fn subscriber(inner: Weak<Inner>, redis: Redis) {
    let mut backoff = MIN_BACKOFF;
    let mut warned = false;
    loop {
        if inner.strong_count() == 0 {
            return;
        }
        match connect(&redis).await {
            Ok((sink, stream)) => {
                backoff = MIN_BACKOFF;
                set_subscribed(&inner, true);
                tracing::info!("event subscription up: events are shared across instances");
                let reason = listen(&inner, sink, stream).await;
                set_subscribed(&inner, false);
                if inner.strong_count() == 0 {
                    return;
                }
                tracing::warn!(
                    reason,
                    "event subscription lost; events stay local until it is back"
                );
                warned = true;
            }
            Err(err) => {
                if warned {
                    tracing::debug!(error = %err, "event subscription failed");
                } else {
                    tracing::warn!(error = %err, "event subscription failed; events stay local (retrying)");
                    warned = true;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn set_subscribed(inner: &Weak<Inner>, up: bool) {
    if let Some(inner) = inner.upgrade() {
        inner.subscribed.store(up, Ordering::Relaxed);
    }
}

async fn connect(
    redis: &Redis,
) -> anyhow::Result<(redis::aio::PubSubSink, redis::aio::PubSubStream)> {
    let attempt = async {
        let pubsub = redis.client().get_async_pubsub().await?;
        let (mut sink, stream) = pubsub.split();
        sink.subscribe(CHANNEL).await?;
        Ok::<_, redis::RedisError>((sink, stream))
    };
    match tokio::time::timeout(CONNECT_TIMEOUT, attempt).await {
        Ok(Ok(pair)) => Ok(pair),
        // The error kind only: never anything that could echo the URL.
        Ok(Err(err)) => Err(anyhow::anyhow!("{:?}", err.kind())),
        Err(_) => Err(anyhow::anyhow!("timed out")),
    }
}

/// Forwards messages until the connection fails; returns why it stopped.
async fn listen(
    inner: &Weak<Inner>,
    mut sink: redis::aio::PubSubSink,
    mut stream: redis::aio::PubSubStream,
) -> &'static str {
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    loop {
        tokio::select! {
            message = stream.next() => {
                let Some(message) = message else {
                    return "connection closed";
                };
                let Some(inner) = inner.upgrade() else {
                    return "shutting down";
                };
                forward(&inner, message.get_payload_bytes());
            }
            _ = ping.tick() => {
                if inner.strong_count() == 0 {
                    return "shutting down";
                }
                match tokio::time::timeout(PING_TIMEOUT, sink.ping::<redis::Value>()).await {
                    Ok(Ok(_)) => {}
                    _ => return "ping failed",
                }
            }
        }
    }
}

fn forward(inner: &Arc<Inner>, payload: &[u8]) {
    let envelope: Envelope = match serde_json::from_slice(payload) {
        Ok(envelope) => envelope,
        Err(err) => {
            tracing::warn!(error = %err, "ignoring an undecodable event from Redis");
            return;
        }
    };
    if envelope.origin == *inner.instance_id {
        return;
    }
    let recipients = envelope
        .recipients
        .iter()
        .filter_map(|id| id.parse::<Uuid>().ok());
    let _ = inner
        .local
        .send(Arc::new(Delivery::new(recipients, envelope.event)));
}
