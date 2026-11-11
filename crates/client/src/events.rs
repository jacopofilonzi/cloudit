//! Following the server's real-time events, so the UI knows when to sync.
//!
//! [`follow`] runs a task that holds `GET /api/v1/events` (SSE) open and turns it into
//! [`RemoteEvent`]s on a channel. Contract (STRUCTURE.md "Real-time events"): on every
//! (re)connect the heads are fetched from `GET /sync/heads` and compared with what the
//! follower knew (initially: each linked folder's base), then events are applied, and
//! `/sync/heads` is also polled every [`POLL_INTERVAL`] with `If-None-Match`, in both
//! real-time modes: with `realtime: local` (no shared fan-out between server instances)
//! it is the only way to see other instances' commits, and with `shared` it is the safety
//! net for an event lost on the way (a failed Redis publish on one instance is dropped,
//! not retried). A 304 costs little. A dropped stream reconnects
//! with exponential backoff (1 s to 60 s). A refused token (`unauthorized`, blocked
//! account) ends the follower with [`RemoteEvent::SignedOut`].
//!
//! Before every (re)connect the follower asks `/health` whether the server works with
//! this app ([`crate::compat`]); a change is reported as [`RemoteEvent::Compatibility`].
//! An incompatible server gets no stream: the follower waits [`COMPAT_RECHECK`] and asks
//! again (no reconnect loop).
//!
//! Head changes are reported only for linked folders; the UI typically answers with
//! [`crate::Engine::status`] or [`crate::Engine::sync`].

use std::collections::HashMap;
use std::time::Duration;

use cloudit_core::protocol::{ConflictEvent, FolderHeadEvent, HelloEvent, RealtimeMode, SyncHeads};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::Engine;
use crate::api::Api;
use crate::compat::Compat;
use crate::error::{Error, ErrorKind, Result};
use crate::progress::{CancellationToken, OpContext};

/// Heads polling interval (in both real-time modes: see the module docs).
pub const POLL_INTERVAL: Duration = Duration::from_secs(300);
/// Longest reconnect backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Largest SSE line kept (longer ones are dropped).
const MAX_LINE: usize = 1024 * 1024;
/// How often an incompatible server is checked again (it may get updated).
pub const COMPAT_RECHECK: Duration = Duration::from_secs(300);

/// What the follower reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RemoteEvent {
    /// The stream is open (again).
    Connected {
        // The core type is exported by its own crate; a literal avoids an import.
        #[cfg_attr(feature = "ts", ts(type = "\"shared\" | \"local\""))]
        realtime: RealtimeMode,
    },
    /// The stream dropped; the follower reconnects.
    Disconnected { error: String },
    /// A linked folder's head (or its count of unresolved conflicts) changed.
    HeadChanged {
        /// Local id of the linked folder.
        folder: i64,
        folder_id: String,
        head_snapshot_id: Option<String>,
    },
    /// A conflict snapshot appeared in a linked folder.
    ConflictCreated {
        folder: i64,
        folder_id: String,
        snapshot_id: String,
    },
    /// Scopes, folders or memberships changed: refresh lists and names.
    FoldersChanged,
    /// Storage usage changed.
    UsageChanged,
    /// The token was refused (expired, revoked, account blocked): sign in again. The
    /// follower stops.
    SignedOut { error_kind: ErrorKind },
    /// The server's compatibility with this app changed (the first report only when it
    /// is incompatible). While incompatible the follower holds no stream and checks
    /// again every [`COMPAT_RECHECK`].
    Compatibility { compat: Compat },
}

/// Known state of a server folder: head and unresolved conflicts (`None` = unknown).
type Known = HashMap<String, (Option<String>, Option<u64>)>;

/// Starts following the events of the signed-in account until `cancel`.
pub fn follow(engine: &Engine, cancel: CancellationToken) -> mpsc::Receiver<RemoteEvent> {
    let (tx, rx) = mpsc::channel(256);
    let engine = engine.clone();
    tokio::spawn(async move {
        let mut follower = Follower {
            engine,
            tx,
            known: HashMap::new(),
            etag: None,
            ctx: OpContext::new().with_cancel(cancel.clone()),
            compat: Compat::Ok,
        };
        follower.run(&cancel).await;
    });
    rx
}

struct Follower {
    engine: Engine,
    tx: mpsc::Sender<RemoteEvent>,
    known: Known,
    etag: Option<String>,
    ctx: OpContext,
    /// The last compatibility answer reported (initially assumed fine).
    compat: Compat,
}

impl Follower {
    async fn run(&mut self, cancel: &CancellationToken) {
        if let Ok(folders) = self.engine.folders() {
            for f in folders {
                self.known.insert(f.folder_id, (f.base_snapshot_id, None));
            }
        }
        let mut backoff = Duration::from_secs(1);
        while !cancel.is_cancelled() {
            let api = match self.engine.api() {
                Ok(api) => api,
                Err(e) => {
                    self.send(RemoteEvent::SignedOut {
                        error_kind: e.kind(),
                    })
                    .await;
                    return;
                }
            };
            let result = match self.engine.check_compatibility(&self.ctx).await {
                Ok(compat) => {
                    let ok = compat.is_ok();
                    if compat != self.compat {
                        self.compat = compat.clone();
                        self.send(RemoteEvent::Compatibility { compat }).await;
                    }
                    if ok {
                        self.stream(&api, &mut backoff).await
                    } else {
                        if cancel
                            .run_until_cancelled(tokio::time::sleep(COMPAT_RECHECK))
                            .await
                            .is_none()
                        {
                            return;
                        }
                        backoff = Duration::from_secs(1);
                        continue;
                    }
                }
                Err(e) => Err(e),
            };
            if cancel.is_cancelled() {
                return;
            }
            let error = match result {
                Ok(()) => "the stream ended".to_owned(),
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::Unauthorized | ErrorKind::AccountBlocked
                    ) =>
                {
                    self.send(RemoteEvent::SignedOut {
                        error_kind: e.kind(),
                    })
                    .await;
                    return;
                }
                Err(e) => e.to_string(),
            };
            tracing::debug!(%error, ?backoff, "event stream dropped");
            self.send(RemoteEvent::Disconnected { error }).await;
            if cancel
                .run_until_cancelled(tokio::time::sleep(backoff))
                .await
                .is_none()
            {
                return;
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    async fn send(&self, event: RemoteEvent) {
        let _closed = self.tx.send(event).await;
    }

    /// Holds one stream until it ends.
    async fn stream(&mut self, api: &Api, backoff: &mut Duration) -> Result<()> {
        let mut response = api.events(&self.ctx).await?;
        let mut line: Vec<u8> = Vec::new();
        let mut event = String::new();
        let mut data = String::new();
        let mut poll = tokio::time::interval(POLL_INTERVAL);
        poll.tick().await;
        loop {
            let chunk = tokio::select! {
                () = self.ctx.cancel_token().cancelled() => return Ok(()),
                _ = poll.tick() => {
                    self.resync(api).await?;
                    continue;
                }
                chunk = response.chunk() => chunk,
            };
            let Some(chunk) = chunk.map_err(|e| Error::Network(format!("event stream: {e}")))?
            else {
                return Ok(());
            };
            for byte in chunk {
                if byte != b'\n' {
                    if line.len() < MAX_LINE {
                        line.push(byte);
                    }
                    continue;
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let text = String::from_utf8_lossy(&line).into_owned();
                line.clear();
                if text.is_empty() {
                    if !event.is_empty() && self.dispatch(api, &event, &data).await?.is_some() {
                        *backoff = Duration::from_secs(1);
                    }
                    event.clear();
                    data.clear();
                } else if let Some(value) = text.strip_prefix("event:") {
                    value.trim().clone_into(&mut event);
                } else if let Some(value) = text.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.strip_prefix(' ').unwrap_or(value));
                }
            }
        }
    }

    /// Handles one event; the real-time mode when it was a `hello`.
    async fn dispatch(
        &mut self,
        api: &Api,
        event: &str,
        data: &str,
    ) -> Result<Option<RealtimeMode>> {
        match event {
            "hello" => {
                let hello: HelloEvent = parse(data)?;
                self.send(RemoteEvent::Connected {
                    realtime: hello.realtime,
                })
                .await;
                self.resync(api).await?;
                return Ok(Some(hello.realtime));
            }
            "folder_head" => {
                let head: FolderHeadEvent = parse(data)?;
                let known = self.known.entry(head.folder_id.clone()).or_default();
                if known.0.as_deref() != Some(head.snapshot_id.as_str()) {
                    known.0 = Some(head.snapshot_id.clone());
                    if let Some(folder) = self.linked(&head.folder_id) {
                        self.send(RemoteEvent::HeadChanged {
                            folder,
                            folder_id: head.folder_id,
                            head_snapshot_id: Some(head.snapshot_id),
                        })
                        .await;
                    }
                }
            }
            "conflict" => {
                let conflict: ConflictEvent = parse(data)?;
                if let Some(folder) = self.linked(&conflict.folder_id) {
                    self.send(RemoteEvent::ConflictCreated {
                        folder,
                        folder_id: conflict.folder_id,
                        snapshot_id: conflict.snapshot_id,
                    })
                    .await;
                }
            }
            "folder_changed" | "scope_changed" | "membership_changed" => {
                self.send(RemoteEvent::FoldersChanged).await;
            }
            "usage_changed" => self.send(RemoteEvent::UsageChanged).await,
            _ => {}
        }
        Ok(None)
    }

    /// Compares `/sync/heads` with what is known; reports the linked folders that moved.
    async fn resync(&mut self, api: &Api) -> Result<()> {
        let Some((heads, etag)) = api.heads(self.etag.as_deref(), &self.ctx).await? else {
            return Ok(());
        };
        self.etag = etag;
        let SyncHeads { folders } = heads;
        for head in folders {
            let known = self.known.entry(head.folder_id.clone()).or_default();
            let moved = known.0 != head.head_snapshot_id
                || known.1.is_some_and(|c| c != head.unresolved_conflicts);
            *known = (
                head.head_snapshot_id.clone(),
                Some(head.unresolved_conflicts),
            );
            if moved && let Some(folder) = self.linked(&head.folder_id) {
                self.send(RemoteEvent::HeadChanged {
                    folder,
                    folder_id: head.folder_id,
                    head_snapshot_id: head.head_snapshot_id,
                })
                .await;
            }
        }
        Ok(())
    }

    /// The local id of a linked server folder.
    fn linked(&self, folder_id: &str) -> Option<i64> {
        self.engine
            .folders()
            .ok()?
            .into_iter()
            .find(|f| f.folder_id == folder_id)
            .map(|f| f.id)
    }
}

fn parse<T: serde::de::DeserializeOwned>(data: &str) -> Result<T> {
    serde_json::from_str(data).map_err(|e| Error::Protocol(format!("event payload: {e}")))
}
