//! History, manifests, round trips, pin and label, restores ("keep local").

use axum::http::{StatusCode, header};
use cloudit_core::manifest::{DEFAULT_MAX_MANIFEST_BYTES, Manifest, decode_manifest};
use sea_orm::{ColumnTrait as _, EntityTrait as _, PaginatorTrait as _, QueryFilter as _};
use serde_json::json;
use uuid::Uuid;

use super::scopes::team;
use super::snapshots::edited;
use super::support::TestApp;
use super::sync_client::{SyncClient, Tree, consistent, usage_of};
use crate::db::entities::snapshot_chunks;

#[tokio::test]
async fn a_snapshot_downloads_byte_identical() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let device = SyncClient::new(&app, &t.device, &t.folder);
    let tree = Tree::sample();
    let first = device.sync(&tree, None).await;
    let changed = edited(&tree);
    let second = device.sync(&changed, Some(&first)).await;

    // Another member, from scratch.
    let viewer = SyncClient::new(&app, &t.viewer, &t.folder);
    assert_eq!(viewer.download(&second).await, changed);
    assert_eq!(viewer.download(&first).await, tree);

    // The JSON form for the web UI, and the ETag.
    let manifest_url = format!("/api/v1/snapshots/{second}/manifest");
    let stored = app.get_bytes(&manifest_url, &t.viewer).await;
    let (manifest, hash) = decode_manifest(&stored.body, DEFAULT_MAX_MANIFEST_BYTES).unwrap();
    let json = app
        .get_bytes(&format!("{manifest_url}?format=json"), &t.viewer)
        .await;
    json.assert(StatusCode::OK);
    assert_eq!(json.headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(json.body, manifest.canonical_json().unwrap());
    let parsed: Manifest = serde_json::from_slice(&json.body).unwrap();
    assert_eq!(parsed, manifest);
    let mut request = app.request("GET", &manifest_url, &t.viewer, None);
    request.headers_mut().insert(
        header::IF_NONE_MATCH,
        format!("\"{hash}\"").parse().unwrap(),
    );
    let cached = app.send_for_bytes(request).await;
    cached.assert(StatusCode::NOT_MODIFIED);
    assert!(cached.body.is_empty());
    app.get_bytes(&format!("{manifest_url}?format=xml"), &t.viewer)
        .await
        .assert(StatusCode::BAD_REQUEST);
    app.get_bytes(&manifest_url, &t.outsider)
        .await
        .assert(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn history_pages_and_filters() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.owner, &t.folder);
    let a = Tree::sample();
    let b = edited(&a);
    let mut ids = Vec::new();
    let mut parent: Option<String> = None;
    for i in 0..5 {
        let tree = if i % 2 == 0 { &a } else { &b };
        let id = client.sync(tree, parent.as_deref()).await;
        parent = Some(id.clone());
        ids.push(id);
    }
    let (manifest, _) = b.manifest();
    let resp = client.commit(&manifest, Some(&ids[0]), "conflict").await;
    resp.assert(StatusCode::CREATED);
    let conflict = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();

    let base = format!("/api/v1/folders/{}/snapshots", t.folder);
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let url = match &cursor {
            Some(c) => format!("{base}?limit=2&cursor={c}"),
            None => format!("{base}?limit=2"),
        };
        let page = app.get(&url, &t.viewer).await;
        page.assert(StatusCode::OK);
        let list = page.body["snapshots"].as_array().unwrap();
        assert!(list.len() <= 2);
        seen.extend(list.iter().map(|s| s["id"].as_str().unwrap().to_owned()));
        match page.body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    let mut expected = ids.clone();
    expected.push(conflict.clone());
    expected.reverse();
    assert_eq!(seen, expected, "newest first, every snapshot once");

    let all = app.get(&base, &t.device).await;
    assert_eq!(all.body["snapshots"].as_array().unwrap().len(), 6);
    assert_eq!(all.body["next_cursor"], json!(null));
    let heads: Vec<bool> = all.body["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["is_head"].as_bool().unwrap())
        .collect();
    assert_eq!(heads, [false, true, false, false, false, false]);

    let only = app.get(&format!("{base}?kind=conflict"), &t.viewer).await;
    assert_eq!(only.body["snapshots"][0]["id"], conflict.as_str());
    assert_eq!(only.body["snapshots"].as_array().unwrap().len(), 1);
    let open = app.get(&format!("{base}?unresolved=true"), &t.viewer).await;
    assert_eq!(open.body["snapshots"].as_array().unwrap().len(), 1);
    app.post(
        &format!("/api/v1/snapshots/{conflict}/resolve"),
        &t.owner,
        json!({}),
    )
    .await
    .assert(StatusCode::OK);
    let open = app.get(&format!("{base}?unresolved=true"), &t.viewer).await;
    assert_eq!(open.body["snapshots"], json!([]));

    for bad in ["limit=0", "limit=201", "cursor=x"] {
        app.get(&format!("{base}?{bad}"), &t.viewer)
            .await
            .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
    app.get(&format!("{base}?other=1"), &t.viewer)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    app.get(&base, &t.outsider)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn pin_and_label() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let id = SyncClient::new(&app, &t.owner, &t.folder)
        .sync(&Tree::sample(), None)
        .await;
    let url = format!("/api/v1/snapshots/{id}");
    let patch = |who, body| app.call("PATCH", &url, who, Some(body));

    let resp = patch(
        &t.device,
        json!({ "pinned": true, "label": "  Before the boss  " }),
    )
    .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["pinned"], true);
    assert_eq!(resp.body["label"], "Before the boss");
    assert_eq!(resp.body["is_head"], true);
    let resp = patch(&t.editor, json!({ "pinned": false })).await;
    assert_eq!(resp.body["pinned"], false);
    assert_eq!(resp.body["label"], "Before the boss", "omitted: unchanged");
    let resp = patch(&t.editor, json!({ "label": null })).await;
    assert_eq!(resp.body["label"], json!(null));
    patch(&t.editor, json!({ "label": "x" }))
        .await
        .assert(StatusCode::OK);
    let resp = patch(&t.editor, json!({ "label": "" })).await;
    assert_eq!(resp.body["label"], json!(null));

    patch(&t.editor, json!({ "label": "x".repeat(101) }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    patch(&t.editor, json!({ "label": "a\u{7}" }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    patch(&t.editor, json!({ "other": 1 }))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    patch(&t.viewer, json!({ "pinned": true }))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    patch(&t.outsider, json!({ "pinned": true }))
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
}

async fn chunk_refs(app: &TestApp, snapshot: &str) -> u64 {
    let id: Uuid = snapshot.parse().unwrap();
    snapshot_chunks::Entity::find()
        .filter(snapshot_chunks::Column::SnapshotId.eq(id))
        .count(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn restore_and_keep_local() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let device = SyncClient::new(&app, &t.device, &t.folder);
    let tree = Tree::sample();
    let first = device.sync(&tree, None).await;
    let second = device.sync(&edited(&tree), Some(&first)).await;
    let usage = consistent(usage_of(&app, &t.scope).await);

    // Restore the first version on top of the second.
    let restore =
        |id: &str, who, body| app.post(&format!("/api/v1/snapshots/{id}/restore"), who, body);
    restore(&first, &t.viewer, json!({}))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    let resp = restore(&first, &t.editor, json!({ "expected_head": first })).await;
    resp.assert_error(StatusCode::CONFLICT, "conflict");
    assert_eq!(resp.body["head_snapshot_id"], second.as_str());
    restore(&first, &t.editor, json!({ "expected_head": "x" }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let resp = restore(&first, &t.editor, json!({ "expected_head": second })).await;
    resp.assert(StatusCode::CREATED);
    let restored = resp.body["id"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["kind"], "restore");
    assert_eq!(resp.body["restored_from"], first.as_str());
    assert_eq!(resp.body["parent_id"], second.as_str());
    assert_eq!(resp.body["is_head"], true);
    assert_eq!(resp.body["device_id"], json!(null), "made with a session");
    assert_eq!(resp.body["device_name"], json!(null));
    assert!(!resp.body["author_name"].as_str().unwrap().is_empty());
    let source = app
        .get(&format!("/api/v1/snapshots/{first}"), &t.viewer)
        .await;
    assert_eq!(resp.body["manifest_hash"], source.body["manifest_hash"]);
    assert_eq!(resp.body["file_count"], source.body["file_count"]);
    assert_eq!(
        chunk_refs(&app, &restored).await,
        chunk_refs(&app, &first).await
    );
    assert_eq!(
        consistent(usage_of(&app, &t.scope).await),
        usage,
        "a restore stores nothing"
    );
    let viewer = SyncClient::new(&app, &t.viewer, &t.folder);
    assert_eq!(viewer.download(&restored).await, tree);

    // Without expected_head: on top of whatever the head is (an empty body is fine).
    let resp = app
        .send_raw(
            "POST",
            &format!("/api/v1/snapshots/{second}/restore"),
            &t.device,
            Vec::new(),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    let latest = resp.body["id"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["parent_id"], restored.as_str());
    assert!(resp.body["device_id"].is_string());

    // Keep local: restoring a conflict snapshot makes it the head and resolves it.
    let mut local = Tree::sample();
    local.file("local.txt", b"mine".to_vec());
    let (manifest, chunks) = local.manifest();
    device.push_chunks(&chunks).await;
    let resp = device.commit(&manifest, Some(&first), "conflict").await;
    let conflict = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    let folder = format!("/api/v1/folders/{}", t.folder);
    assert_eq!(
        app.get(&folder, &t.viewer).await.body["unresolved_conflicts"],
        1
    );
    let resp = restore(&conflict, &t.device, json!({ "expected_head": latest })).await;
    resp.assert(StatusCode::CREATED);
    let kept = resp.body["id"].as_str().unwrap().to_owned();
    let folder_now = app.get(&folder, &t.viewer).await;
    assert_eq!(folder_now.body["unresolved_conflicts"], 0);
    assert_eq!(folder_now.body["head_snapshot_id"], kept.as_str());
    let source = app
        .get(&format!("/api/v1/snapshots/{conflict}"), &t.viewer)
        .await;
    assert!(source.body["conflict_resolved_at"].is_string());
    assert_eq!(viewer.download(&kept).await, local);

    // Normal commits continue on top of the restore.
    device.sync(&tree, Some(&kept)).await;
}
