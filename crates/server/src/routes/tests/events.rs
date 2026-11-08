//! Real-time events: the SSE stream (auth, hello, recipients, each event from its
//! handler, keep-alive, revocation, per-user limit), Redis fan-out between two
//! instances (`TEST_REDIS_URL`) and the degraded mode, `/sync/heads` with its ETag.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::scopes::{Team, create_folder, id_of, team};
use super::support::{As, TestApp};
use super::sync_client::{SyncClient, Tree};

/// One SSE message.
#[derive(Debug, Clone)]
pub struct Msg {
    pub event: String,
    pub data: Value,
    pub comment: bool,
}

/// What [`Sse::next`] saw.
#[derive(Debug)]
pub enum Next {
    Msg(Msg),
    End,
    Timeout,
}

/// An open event stream read frame by frame.
pub struct Sse {
    body: Body,
    buf: String,
}

const WAIT: Duration = Duration::from_secs(5);

impl Sse {
    pub async fn open(app: &Router, request: Request<Body>) -> Result<Self, (StatusCode, Value)> {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        if status != StatusCode::OK {
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            return Err((
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            ));
        }
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        Ok(Self {
            body: response.into_body(),
            buf: String::new(),
        })
    }

    fn parse(block: &str) -> Msg {
        let mut msg = Msg {
            event: String::new(),
            data: Value::Null,
            comment: false,
        };
        for line in block.lines() {
            if let Some(name) = line.strip_prefix("event:") {
                msg.event = name.trim().to_owned();
            } else if let Some(data) = line.strip_prefix("data:") {
                msg.data = serde_json::from_str(data.trim()).unwrap();
            } else if line.starts_with(':') {
                msg.comment = true;
            }
        }
        msg
    }

    pub async fn next(&mut self, within: Duration) -> Next {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if let Some(at) = self.buf.find("\n\n") {
                let block: String = self.buf.drain(..at + 2).collect();
                return Next::Msg(Self::parse(&block));
            }
            match tokio::time::timeout_at(deadline, self.body.frame()).await {
                Err(_) => return Next::Timeout,
                Ok(None | Some(Err(_))) => return Next::End,
                Ok(Some(Ok(frame))) => {
                    if let Ok(data) = frame.into_data() {
                        self.buf.push_str(&String::from_utf8_lossy(&data));
                    }
                }
            }
        }
    }

    /// The next event (keep-alives skipped).
    pub async fn event(&mut self) -> Msg {
        loop {
            match self.next(WAIT).await {
                Next::Msg(msg) if msg.comment => {}
                Next::Msg(msg) => return msg,
                other => panic!("expected an event, got {other:?}"),
            }
        }
    }

    /// The next event, which must be `name`; its data.
    pub async fn expect(&mut self, name: &str) -> Value {
        let msg = self.event().await;
        assert_eq!(msg.event, name, "{msg:?}");
        msg.data
    }

    /// Asserts nothing but keep-alives arrives for `within`.
    pub async fn quiet(&mut self, within: Duration) {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.next(left).await {
                Next::Timeout => return,
                Next::Msg(msg) if msg.comment => {}
                other => panic!("expected nothing, got {other:?}"),
            }
        }
    }

    /// Waits for the end of the stream (keep-alives skipped).
    pub async fn ended(&mut self) {
        loop {
            match self.next(WAIT).await {
                Next::End => return,
                Next::Msg(_) => {}
                Next::Timeout => panic!("the stream did not end"),
            }
        }
    }
}

async fn open(app: &TestApp, who: &As) -> Sse {
    let sse = Sse::open(&app.app, app.request("GET", "/api/v1/events", who, None))
        .await
        .unwrap();
    with_hello(sse).await
}

async fn with_hello(mut sse: Sse) -> Sse {
    let hello = sse.expect("hello").await;
    assert!(hello["instance_id"].is_string());
    assert!(hello["server_time"].is_string());
    sse
}

#[tokio::test]
async fn stream_needs_credentials_and_starts_with_hello() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let err = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &As::Anon, None),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    let mut sse = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.device, None),
    )
    .await
    .unwrap();
    let hello = sse.expect("hello").await;
    assert_eq!(hello["realtime"], "local");
    assert_eq!(hello["instance_id"], app.state.events.instance_id());
}

#[tokio::test]
async fn only_members_receive_scope_events() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mut owner = open(&app, &t.owner).await;
    let mut viewer = open(&app, &t.viewer).await;
    let mut outsider = open(&app, &t.outsider).await;
    let resp = create_folder(&app, &t.editor, &t.scope, json!({ "name": "New" })).await;
    resp.assert(StatusCode::CREATED);
    let folder = id_of(&resp);
    for sse in [&mut owner, &mut viewer] {
        let data = sse.expect("folder_changed").await;
        assert_eq!(
            data,
            json!({ "scope_id": t.scope, "folder_id": folder, "change": "created" })
        );
    }
    outsider.quiet(Duration::from_millis(300)).await;
}

async fn folder_event(sse: &mut Sse, change: &str) {
    let data = sse.expect("folder_changed").await;
    assert_eq!(data["change"], change, "{data}");
}

async fn scope_event(sse: &mut Sse, change: &str) {
    let data = sse.expect("scope_changed").await;
    assert_eq!(data["change"], change, "{data}");
}

#[tokio::test]
async fn sync_handlers_publish_their_events() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mut sse = open(&app, &t.viewer).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);

    let first = client.sync(&Tree::sample(), None).await;
    let data = sse.expect("folder_head").await;
    assert_eq!(
        data,
        json!({ "scope_id": t.scope, "folder_id": t.folder, "snapshot_id": first })
    );

    // A conflict (stale parent) and its resolution.
    let mut tree = Tree::sample();
    tree.file("new.txt", b"x".to_vec());
    let second = client.sync(&tree, Some(&first)).await;
    sse.expect("folder_head").await;
    tree.file("other.txt", b"y".to_vec());
    let (manifest, chunks) = tree.manifest();
    client.push_chunks(&chunks).await;
    let resp = client.commit(&manifest, Some(&first), "conflict").await;
    resp.assert(StatusCode::CREATED);
    let conflict = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    let data = sse.expect("conflict").await;
    assert_eq!(data["snapshot_id"], conflict.as_str());

    let snap = |id: &str| format!("/api/v1/snapshots/{id}");
    app.call(
        "PATCH",
        &snap(&second),
        &t.editor,
        Some(json!({ "pinned": true })),
    )
    .await
    .assert(StatusCode::OK);
    assert_eq!(
        sse.expect("history_changed").await,
        json!({ "scope_id": t.scope, "folder_id": t.folder })
    );
    app.post(
        &format!("{}/resolve", snap(&conflict)),
        &t.editor,
        json!({}),
    )
    .await
    .assert(StatusCode::OK);
    sse.expect("history_changed").await;
    let resp = app
        .post(&format!("{}/restore", snap(&first)), &t.editor, json!({}))
        .await;
    resp.assert(StatusCode::CREATED);
    let data = sse.expect("folder_head").await;
    assert_eq!(data["snapshot_id"], resp.body["id"]);
}

#[tokio::test]
async fn scope_and_folder_changes_are_published() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mut sse = open(&app, &t.viewer).await;
    let folder = format!("/api/v1/folders/{}", t.folder);
    app.call(
        "PATCH",
        &folder,
        &t.editor,
        Some(json!({ "name": "Renamed" })),
    )
    .await
    .assert(StatusCode::OK);
    folder_event(&mut sse, "updated").await;
    app.call("DELETE", &folder, &t.admin, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    folder_event(&mut sse, "trashed").await;
    app.post(&format!("{folder}/restore"), &t.admin, json!({}))
        .await
        .assert(StatusCode::OK);
    folder_event(&mut sse, "restored").await;

    let scope = format!("/api/v1/scopes/{}", t.scope);
    app.call("PATCH", &scope, &t.owner, Some(json!({ "name": "Other" })))
        .await
        .assert(StatusCode::OK);
    scope_event(&mut sse, "updated").await;
    app.call("DELETE", &scope, &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    scope_event(&mut sse, "trashed").await;
    app.post(&format!("{scope}/restore"), &t.owner, json!({}))
        .await
        .assert(StatusCode::OK);
    scope_event(&mut sse, "restored").await;

    // A new scope: only its owner hears about it.
    let mut owner = open(&app, &t.owner).await;
    app.post("/api/v1/scopes", &t.owner, json!({ "name": "Mine" }))
        .await
        .assert(StatusCode::CREATED);
    scope_event(&mut owner, "created").await;
    sse.quiet(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn membership_changes_reach_members_and_the_affected_user() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mut viewer = open(&app, &t.viewer).await;
    let mut outsider = open(&app, &t.outsider).await;
    let members = format!("/api/v1/scopes/{}/members", t.scope);
    let outsider_id = app.user_id(&t.outsider).await;
    let expected = json!({ "scope_id": t.scope });

    app.post(
        &members,
        &t.owner,
        json!({ "email": "outsider@example.com", "role": "viewer" }),
    )
    .await
    .assert(StatusCode::CREATED);
    assert_eq!(viewer.expect("membership_changed").await, expected);
    assert_eq!(outsider.expect("membership_changed").await, expected);

    let member = format!("{members}/{outsider_id}");
    app.call(
        "PATCH",
        &member,
        &t.owner,
        Some(json!({ "role": "editor" })),
    )
    .await
    .assert(StatusCode::OK);
    viewer.expect("membership_changed").await;
    outsider.expect("membership_changed").await;

    // Removed: the removed user still hears it, then nothing more of this scope.
    app.call("DELETE", &member, &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    viewer.expect("membership_changed").await;
    outsider.expect("membership_changed").await;
    app.call(
        "PATCH",
        &format!("/api/v1/scopes/{}", t.scope),
        &t.owner,
        Some(json!({ "name": "Renamed" })),
    )
    .await
    .assert(StatusCode::OK);
    viewer.expect("scope_changed").await;
    outsider.quiet(Duration::from_millis(300)).await;

    // Leaving.
    let viewer_id = app.user_id(&t.viewer).await;
    let mut owner = open(&app, &t.owner).await;
    app.call("DELETE", &format!("{members}/{viewer_id}"), &t.viewer, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    owner.expect("membership_changed").await;
    viewer.expect("membership_changed").await;
}

#[tokio::test]
async fn keep_alives_and_revocation_end_the_stream() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    app.state.events.set_keepalive(Duration::from_millis(100));

    let mut device = open(&app, &t.device).await;
    match device.next(Duration::from_secs(2)).await {
        Next::Msg(msg) => assert!(msg.comment, "{msg:?}"),
        other => panic!("expected a keep-alive, got {other:?}"),
    }
    let devices = app.get("/api/v1/devices", &t.owner).await;
    let id = devices.body[0]["id"].as_str().unwrap().to_owned();
    app.call("DELETE", &format!("/api/v1/devices/{id}"), &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    device.ended().await;

    // Disabling the account ends its session streams.
    let mut editor = open(&app, &t.editor).await;
    let editor_id = app.user_id(&t.editor).await;
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{editor_id}"),
        &t.site_admin,
        Some(json!({ "disabled": true })),
    )
    .await
    .assert(StatusCode::OK);
    editor.ended().await;

    // Signing out ends it too.
    let mut viewer = open(&app, &t.viewer).await;
    app.post("/api/v1/auth/logout", &t.viewer, json!({}))
        .await
        .assert(StatusCode::NO_CONTENT);
    viewer.ended().await;
}

#[tokio::test]
async fn streams_per_user_are_limited() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mut open_streams = Vec::new();
    for _ in 0..crate::events::MAX_STREAMS_PER_USER {
        open_streams.push(open(&app, &t.viewer).await);
    }
    let err = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(err.1["code"], "rate_limited");
    // Other users are not affected; closing one frees a slot.
    open(&app, &t.editor).await;
    open_streams.pop();
    open(&app, &t.viewer).await;
}

#[tokio::test]
async fn redis_down_degrades_to_local_delivery() {
    let app = TestApp::with(&[("REDIS_URL", "redis://127.0.0.1:1")]).await;
    let t = team(&app).await;
    let mut sse = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .unwrap();
    assert_eq!(sse.expect("hello").await["realtime"], "local");
    create_folder(&app, &t.editor, &t.scope, json!({ "name": "Local" }))
        .await
        .assert(StatusCode::CREATED);
    folder_event(&mut sse, "created").await;
}

async fn wait_shared(state: &crate::state::AppState) {
    for _ in 0..100 {
        if state.events.realtime() == cloudit_core::protocol::RealtimeMode::Shared {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the Redis subscription did not come up");
}

#[tokio::test]
async fn events_cross_instances_through_redis() {
    let Ok(url) = std::env::var("TEST_REDIS_URL") else {
        eprintln!("skipped: set TEST_REDIS_URL (make test-ha) to run against Redis");
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let app = TestApp::with(&[("REDIS_URL", url.as_str())]).await;
    let t: Team = team(&app).await;
    let (other, other_app) = app.second_instance().await;
    wait_shared(&app.state).await;
    wait_shared(&other).await;

    // A stream on the second instance hears a change made through the first.
    let mut sse = Sse::open(
        &other_app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .unwrap();
    let hello = sse.expect("hello").await;
    assert_eq!(hello["realtime"], "shared");
    assert_eq!(hello["instance_id"], other.events.instance_id());
    let mut outsider = Sse::open(
        &other_app,
        app.request("GET", "/api/v1/events", &t.outsider, None),
    )
    .await
    .unwrap();
    outsider.expect("hello").await;
    let mut local = open(&app, &t.viewer).await;

    let resp = create_folder(&app, &t.editor, &t.scope, json!({ "name": "Shared" })).await;
    resp.assert(StatusCode::CREATED);
    let data = sse.expect("folder_changed").await;
    assert_eq!(data["folder_id"], id_of(&resp).as_str());
    // Delivered once locally (the instance ignores its own messages from Redis).
    local.expect("folder_changed").await;
    local.quiet(Duration::from_millis(500)).await;
    outsider.quiet(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn heads_list_visible_folders_with_an_etag() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let resp = app.get("/api/v1/sync/heads", &t.device).await;
    resp.assert(StatusCode::OK);
    assert_eq!(
        resp.body,
        json!({ "folders": [{
            "scope_id": t.scope, "folder_id": t.folder, "head_snapshot_id": null,
            "head_created_at": null, "unresolved_conflicts": 0
        }] })
    );
    let etag = resp.headers[header::ETAG].to_str().unwrap().to_owned();
    let mut request = app.request("GET", "/api/v1/sync/heads", &t.device, None);
    request
        .headers_mut()
        .insert(header::IF_NONE_MATCH, etag.parse().unwrap());
    app.send(request).await.assert(StatusCode::NOT_MODIFIED);

    let head = SyncClient::new(&app, &t.editor, &t.folder)
        .sync(&Tree::sample(), None)
        .await;
    let resp = app.get("/api/v1/sync/heads", &t.viewer).await;
    assert_eq!(resp.body["folders"][0]["head_snapshot_id"], head.as_str());
    assert!(resp.body["folders"][0]["head_created_at"].is_string());
    assert_ne!(resp.headers[header::ETAG].to_str().unwrap(), etag);

    // Outsiders see nothing; trashed folders disappear.
    let resp = app.get("/api/v1/sync/heads", &t.outsider).await;
    assert_eq!(resp.body, json!({ "folders": [] }));
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{}", t.folder),
        &t.admin,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    let resp = app.get("/api/v1/sync/heads", &t.viewer).await;
    assert_eq!(resp.body, json!({ "folders": [] }));
    app.get("/api/v1/sync/heads", &As::Anon)
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}
