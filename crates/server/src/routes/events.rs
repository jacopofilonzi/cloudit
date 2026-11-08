//! `GET /api/v1/events` (Server-Sent Events) and `GET /api/v1/sync/heads` (the
//! polling fallback and the resync after a reconnect). Session or device token.
//!
//! A stream sends `hello` first, then the events addressed to its user (see
//! `crate::events`), and a keep-alive comment every [`crate::events::KEEPALIVE`]. At each
//! keep-alive it re-checks its credentials (session or device token still valid, user
//! enabled and approved) and ends otherwise; it also ends when it falls too far behind.

use std::convert::Infallible;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse as _, Response};
use cloudit_core::protocol::{HelloEvent, RealtimeMode, ServerEvent, SyncHead, SyncHeads};
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use sha2::{Digest as _, Sha256};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::accounts::users;
use crate::auth::extractors::AnyUser;
use crate::db::entities::{devices, folders, sessions};
use crate::errors::ApiError;
use crate::events::{Delivery, StreamSlot};
use crate::scopes::store;
use crate::state::AppState;
use crate::sync::snapshots;
use crate::time::rfc3339;

/// The credentials a stream was opened with.
#[derive(Debug, Clone, Copy)]
enum Credential {
    Session(Uuid),
    Device(Uuid),
}

/// Whether the stream's credentials still work (see the module docs).
async fn still_valid(state: &AppState, user: Uuid, credential: Credential) -> anyhow::Result<bool> {
    let now = crate::time::now();
    let alive = match credential {
        Credential::Session(id) => sessions::Entity::find_by_id(id)
            .filter(sessions::Column::ExpiresAt.gt(now))
            .one(&state.db)
            .await?
            .is_some(),
        Credential::Device(id) => devices::Entity::find_by_id(id)
            .filter(devices::Column::RevokedAt.is_null())
            .filter(devices::Column::ExpiresAt.gt(now))
            .one(&state.db)
            .await?
            .is_some(),
    };
    Ok(alive
        && users::find(&state.db, user)
            .await?
            .is_some_and(|u| u.is_active()))
}

fn sse(event: &ServerEvent) -> Option<Event> {
    match event.data_json() {
        Ok(data) => Some(Event::default().event(event.name()).data(data)),
        Err(err) => {
            tracing::warn!(error = %err, event = event.name(), "encoding an event");
            None
        }
    }
}

fn hello(state: &AppState, realtime: RealtimeMode) -> Event {
    let event = ServerEvent::Hello(HelloEvent {
        server_time: rfc3339(crate::time::now()),
        instance_id: state.events.instance_id().to_owned(),
        realtime,
    });
    sse(&event).unwrap_or_default()
}

struct Stream {
    state: AppState,
    user: Uuid,
    credential: Credential,
    rx: broadcast::Receiver<std::sync::Arc<Delivery>>,
    tick: tokio::time::Interval,
    /// The mode announced by the last `hello` (`None` before the first).
    mode: Option<RealtimeMode>,
    _slot: StreamSlot,
}

impl Stream {
    /// The next SSE message, or `None` to end the stream.
    async fn next(&mut self) -> Option<Event> {
        if self.mode.is_none() {
            let mode = self.state.events.realtime();
            self.mode = Some(mode);
            return Some(hello(&self.state, mode));
        }
        loop {
            tokio::select! {
                received = self.rx.recv() => match received {
                    Ok(delivery) if delivery.is_for(self.user) => {
                        if let Some(event) = sse(&delivery.event) {
                            return Some(event);
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(missed)) => {
                        tracing::debug!(user_id = %self.user, missed, "event stream lagged; closing it");
                        return None;
                    }
                    Err(RecvError::Closed) => return None,
                },
                _ = self.tick.tick() => {
                    match still_valid(&self.state, self.user, self.credential).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::debug!(user_id = %self.user, "event stream credentials gone; closing it");
                            return None;
                        }
                        // A transient database error doesn't end the stream: the next
                        // keep-alive checks again.
                        Err(err) => tracing::warn!(error = %format!("{err:#}"), "re-checking event stream credentials"),
                    }
                    let mode = self.state.events.realtime();
                    if self.mode != Some(mode) {
                        self.mode = Some(mode);
                        return Some(hello(&self.state, mode));
                    }
                    return Some(Event::default().comment("keep-alive"));
                }
            }
        }
    }
}

/// `GET /events` (session or device token): the caller's event stream.
pub async fn stream(State(state): State<AppState>, me: AnyUser) -> Result<Response, ApiError> {
    let credential = match (me.session_id, me.device_id) {
        (Some(session), _) => Credential::Session(session),
        (None, Some(device)) => Credential::Device(device),
        (None, None) => return Err(ApiError::Unauthorized),
    };
    let slot = state.events.open_stream(me.user.id)?;
    let every = state.events.keepalive();
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let stream = Stream {
        rx: state.events.subscribe(),
        tick,
        user: me.user.id,
        credential,
        state,
        mode: None,
        _slot: slot,
    };
    let body = futures::stream::unfold(stream, |mut s| async move {
        s.next().await.map(|event| (Ok::<_, Infallible>(event), s))
    });
    Ok((
        // Reverse proxies (nginx) must not buffer the stream.
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        Sse::new(body),
    )
        .into_response())
}

/// `GET /sync/heads` (session or device token): every folder the caller can see
/// outside the trash with its head and open conflicts, by folder id. `ETag` = a hash of
/// the body; `If-None-Match` with it → 304.
pub async fn heads(
    State(state): State<AppState>,
    me: AnyUser,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let scopes = store::list_visible(&state.db, me.user.id).await?;
    let ids: Vec<Uuid> = scopes.iter().map(|(s, _)| s.id).collect();
    let list = if ids.is_empty() {
        Vec::new()
    } else {
        let mut list = folders::Entity::find()
            .filter(folders::Column::ScopeId.is_in(ids))
            .filter(folders::Column::DeletedAt.is_null())
            .all(&state.db)
            .await
            .map_err(anyhow::Error::new)?;
        list.sort_by_key(|f| f.id);
        list
    };
    let sync = snapshots::folder_sync(&state.db, &list.iter().collect::<Vec<_>>()).await?;
    let body = SyncHeads {
        folders: list
            .iter()
            .map(|f| {
                let s = sync.get(&f.id);
                SyncHead {
                    scope_id: f.scope_id.to_string(),
                    folder_id: f.id.to_string(),
                    head_snapshot_id: f.head_snapshot_id.map(|id| id.to_string()),
                    head_created_at: s
                        .and_then(|s| s.head.as_ref())
                        .map(|h| rfc3339(h.created_at)),
                    unresolved_conflicts: s.map_or(0, |s| s.unresolved_conflicts),
                }
            })
            .collect(),
    };
    let json = serde_json::to_vec(&body).map_err(anyhow::Error::new)?;
    let etag = format!("\"{}\"", hex::encode(&Sha256::digest(&json)[..16]));
    let cache = [
        (header::ETAG, etag.clone()),
        (header::CACHE_CONTROL, "private, no-cache".to_owned()),
    ];
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    Ok((cache, [(header::CONTENT_TYPE, "application/json")], json).into_response())
}
