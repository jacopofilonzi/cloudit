//! Garbage collection and the orphan sweep: what they delete, usage kept equal to
//! `usage::recompute`, and the interleavings with uploads, adoptions and commits forced
//! with `hooks` (the invariant: a row always has its object).

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cloudit_core::chunk::codec::encode_chunk;
use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::{JobName, JobOutcome};
use http_body_util::BodyExt as _;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::events::Sse;
use super::jobs::run_now;
use super::scopes::{Team, team};
use super::support::{As, TestApp};
use super::sync_client::{SyncClient, Tree, consistent, pseudo_random, usage_of};
use crate::db::entities::{chunks, manifests, snapshots};
use crate::hooks::{self, Point};
use crate::jobs::retention;
use crate::jobs::scheduler::{self, Attempt};
use crate::sync::keys;

const DAY: Duration = Duration::from_secs(24 * 3600);

/// Every chunk and manifest row of the scope has its object; usage matches the rows.
async fn assert_consistent(app: &TestApp, scope: &str) {
    let id: Uuid = scope.parse().unwrap();
    for row in chunks::Entity::find()
        .filter(chunks::Column::ScopeId.eq(id))
        .all(&app.state.db)
        .await
        .unwrap()
    {
        let key = keys::chunk(id, &row.hash.parse().unwrap());
        assert!(
            app.state.blobs.head(&key).await.unwrap().is_some(),
            "chunk row without object: {key}"
        );
    }
    for row in manifests::Entity::find()
        .filter(manifests::Column::ScopeId.eq(id))
        .all(&app.state.db)
        .await
        .unwrap()
    {
        let key = keys::manifest(id, &row.hash.parse().unwrap());
        assert!(
            app.state.blobs.head(&key).await.unwrap().is_some(),
            "manifest row without object: {key}"
        );
    }
    consistent(usage_of(app, scope).await);
}

async fn has_row(app: &TestApp, scope: &str, hash: &ChunkHash) -> bool {
    crate::sync::chunks::exists(&app.state.db, scope.parse().unwrap(), hash)
        .await
        .unwrap()
}

async fn has_object(app: &TestApp, scope: &str, hash: &ChunkHash) -> bool {
    app.state
        .blobs
        .head(&keys::chunk(scope.parse().unwrap(), hash))
        .await
        .unwrap()
        .is_some()
}

/// Stores a chunk object without row, as a presigned upload does.
async fn put_object(app: &TestApp, scope: &str, data: &[u8]) -> ChunkHash {
    let hash = ChunkHash::of(data);
    app.state
        .blobs
        .put(
            &keys::chunk(scope.parse().unwrap(), &hash),
            encode_chunk(data).unwrap().into(),
        )
        .await
        .unwrap();
    hash
}

fn upload_request(app: &TestApp, t: &Team, data: &[u8]) -> Request<Body> {
    let hash = ChunkHash::of(data);
    app.raw_request(
        "PUT",
        &format!("/api/v1/folders/{}/chunks/{hash}", t.folder),
        &t.editor,
        "application/octet-stream",
        encode_chunk(data).unwrap(),
    )
}

/// Sends a request on a task; its status and JSON body.
fn spawn_send(
    app: &TestApp,
    request: Request<Body>,
) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let router = app.app.clone();
    tokio::spawn(async move {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    })
}

fn spawn_job(app: &TestApp, job: JobName, grace: Duration) -> tokio::task::JoinHandle<Attempt> {
    let state = app.state.clone();
    tokio::spawn(async move {
        scheduler::run_leased(&state, job, true, grace)
            .await
            .unwrap()
    })
}

fn succeeded(attempt: &Attempt) -> &crate::jobs::Counts {
    match attempt {
        Attempt::Ran {
            outcome: JobOutcome::Succeeded,
            counts,
            ..
        } => counts,
        other => panic!("job did not succeed: {other:?}"),
    }
}

/// A single-file tree whose file is one chunk.
fn one_chunk_tree(data: &[u8]) -> Tree {
    let mut tree = Tree::default();
    tree.file("file.bin", data.to_vec());
    tree
}

#[tokio::test]
async fn gc_deletes_only_unreferenced_rows_past_the_grace_period() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let old = Tree::sample();
    let first = client.sync(&old, None).await;
    let mut new = Tree::sample();
    new.file("saves/slot1.sav", pseudo_random(99, 2 * 1024 * 1024));
    let head = client.sync(&new, Some(&first)).await;
    let before = consistent(usage_of(&app, &t.scope).await);

    // Everything is referenced: nothing to collect.
    let attempt = scheduler::run_leased(&app.state, JobName::Gc, true, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(succeeded(&attempt).get("chunks_deleted"), 0);

    // The first version goes: its own chunks and manifest become garbage, but only
    // once past the grace period.
    assert!(
        retention::delete_snapshot(&app.state.db, first.parse().unwrap())
            .await
            .unwrap()
    );
    let attempt = scheduler::run_leased(&app.state, JobName::Gc, true, DAY)
        .await
        .unwrap();
    assert_eq!(succeeded(&attempt).get("chunks_deleted"), 0);

    let mut owner = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.owner, None),
    )
    .await
    .unwrap();
    owner.expect("hello").await;
    let counts = run_now(&app.state, JobName::Gc).await;
    let (old_manifest, old_chunks) = old.manifest();
    let (_, new_chunks) = new.manifest();
    let only_old: Vec<&ChunkHash> = old_chunks
        .keys()
        .filter(|h| !new_chunks.contains_key(h))
        .collect();
    assert!(!only_old.is_empty());
    assert_eq!(counts.get("chunks_deleted"), only_old.len() as u64);
    assert_eq!(counts.get("manifests_deleted"), 1);
    assert_eq!(counts.get("objects_deleted"), only_old.len() as u64 + 1);
    for hash in only_old {
        assert!(!has_row(&app, &t.scope, hash).await);
        assert!(!has_object(&app, &t.scope, hash).await);
    }
    let old_key = keys::manifest(t.scope.parse().unwrap(), &old_manifest.hash().unwrap());
    assert!(app.state.blobs.head(&old_key).await.unwrap().is_none());
    let after = consistent(usage_of(&app, &t.scope).await);
    assert!(after < before);
    assert_eq!(counts.get("bytes_freed"), before - after);
    owner.expect("usage_changed").await;
    assert_consistent(&app, &t.scope).await;
    // The head is intact.
    assert_eq!(client.download(&head).await, new);
    // Nothing left to do.
    assert_eq!(
        run_now(&app.state, JobName::Gc)
            .await
            .get("objects_deleted"),
        0
    );
}

#[tokio::test]
async fn upload_during_gc_keeps_its_object() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(1, 50_000);
    let hash = ChunkHash::of(&data);
    app.send(upload_request(&app, &t, &data))
        .await
        .assert(StatusCode::NO_CONTENT);

    // GC deleted the (unreferenced) row and is about to delete the object…
    let pause = hooks::arm(Point::GcRowsDeleted, &t.scope);
    let gc = spawn_job(&app, JobName::Gc, Duration::ZERO);
    pause.reached().await;
    assert!(!has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    // …when the chunk is uploaded again: its row is back.
    app.send(upload_request(&app, &t, &data))
        .await
        .assert(StatusCode::NO_CONTENT);
    pause.release();
    let attempt = gc.await.unwrap();
    assert_eq!(succeeded(&attempt).get("objects_deleted"), 0);
    assert!(has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn gc_deleting_the_object_of_a_paused_upload() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(2, 50_000);
    let hash = ChunkHash::of(&data);
    app.send(upload_request(&app, &t, &data))
        .await
        .assert(StatusCode::NO_CONTENT);
    let gc_pause = hooks::arm(Point::GcRowsDeleted, &t.scope);
    let gc = spawn_job(&app, JobName::Gc, Duration::ZERO);
    gc_pause.reached().await;
    // An upload stores the object (no row now) and pauses before its row…
    let upload_pause = hooks::arm(Point::UploadStored, &t.scope);
    let upload = spawn_send(&app, upload_request(&app, &t, &data));
    upload_pause.reached().await;
    // …GC deletes the object of its tombstone (no row exists)…
    gc_pause.release();
    let attempt = gc.await.unwrap();
    assert_eq!(succeeded(&attempt).get("objects_deleted"), 1);
    assert!(!has_object(&app, &t.scope, &hash).await);
    // …and the upload, under the scope lock, stores it again before inserting.
    upload_pause.release();
    assert_eq!(upload.await.unwrap().0, StatusCode::NO_CONTENT);
    assert!(has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn gc_deleting_the_object_of_a_paused_pack_upload() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(12, 50_000);
    let hash = ChunkHash::of(&data);
    let mut writer = cloudit_core::pack::PackWriter::new();
    writer.push(&hash, &encode_chunk(&data).unwrap()).unwrap();
    let pack = writer.finish();
    let pack_request = || {
        app.raw_request(
            "POST",
            &format!("/api/v1/folders/{}/chunks/pack", t.folder),
            &t.editor,
            cloudit_core::pack::PACK_CONTENT_TYPE,
            pack.clone(),
        )
    };
    app.send(pack_request()).await.assert(StatusCode::OK);
    let gc_pause = hooks::arm(Point::GcRowsDeleted, &t.scope);
    let gc = spawn_job(&app, JobName::Gc, Duration::ZERO);
    gc_pause.reached().await;
    // The pack goes through the same write path as a single upload: paused with its
    // object stored and no row, GC deletes the object, the upload stores it again.
    let upload_pause = hooks::arm(Point::UploadStored, &t.scope);
    let upload = spawn_send(&app, pack_request());
    upload_pause.reached().await;
    gc_pause.release();
    let attempt = gc.await.unwrap();
    assert_eq!(succeeded(&attempt).get("objects_deleted"), 1);
    assert!(!has_object(&app, &t.scope, &hash).await);
    upload_pause.release();
    let (status, body) = upload.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "chunks": 1, "stored": 1 }));
    assert!(has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn sweep_deleting_the_object_of_a_paused_upload() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(3, 50_000);
    let hash = ChunkHash::of(&data);
    let pause = hooks::arm(Point::UploadStored, &t.scope);
    let upload = spawn_send(&app, upload_request(&app, &t, &data));
    pause.reached().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let counts = run_now(&app.state, JobName::OrphanSweep).await;
    assert_eq!(counts.get("objects_deleted"), 1);
    assert!(!has_object(&app, &t.scope, &hash).await);
    pause.release();
    assert_eq!(upload.await.unwrap().0, StatusCode::NO_CONTENT);
    assert!(has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn upload_while_the_sweep_is_listing_keeps_its_object() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(4, 50_000);
    let hash = put_object(&app, &t.scope, &data).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let pause = hooks::arm(Point::SweepListed, &t.scope);
    let sweep = spawn_job(&app, JobName::OrphanSweep, Duration::ZERO);
    pause.reached().await;
    // The object found without row gets one now.
    app.send(upload_request(&app, &t, &data))
        .await
        .assert(StatusCode::NO_CONTENT);
    pause.release();
    let attempt = sweep.await.unwrap();
    assert_eq!(succeeded(&attempt).get("objects_deleted"), 0);
    assert!(has_row(&app, &t.scope, &hash).await);
    assert!(has_object(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn sweep_during_an_adoption_reports_the_chunk_missing() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(5, 50_000);
    let hash = put_object(&app, &t.scope, &data).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let tree = one_chunk_tree(&data);
    let (manifest, chunks) = tree.manifest();
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let commit_uri = client.url("/snapshots?parent=none&kind=normal");
    let commit = |app: &TestApp| {
        app.raw_request(
            "POST",
            &commit_uri,
            &t.editor,
            "application/octet-stream",
            manifest.encode().unwrap().compressed,
        )
    };

    // The commit verified the object (presigned upload) and pauses before adopting it;
    // the sweep deletes it meanwhile.
    let pause = hooks::arm(Point::AdoptVerified, &t.scope);
    let committing = spawn_send(&app, commit(&app));
    pause.reached().await;
    run_now(&app.state, JobName::OrphanSweep).await;
    pause.release();
    let (status, body) = committing.await.unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "chunk_missing");
    assert_eq!(body["hashes"], json!([hash.to_string()]));
    assert!(!has_row(&app, &t.scope, &hash).await);
    assert_consistent(&app, &t.scope).await;
    // The client uploads again and commits.
    client.push_chunks(&chunks).await;
    app.send(commit(&app)).await.assert(StatusCode::CREATED);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn gc_during_a_commit_reports_the_chunks_missing() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let tree = one_chunk_tree(&pseudo_random(6, 50_000));
    let (manifest, chunks) = tree.manifest();
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    client.push_chunks(&chunks).await;
    let commit = || {
        app.raw_request(
            "POST",
            &client.url("/snapshots?parent=none&kind=normal"),
            &t.editor,
            "application/octet-stream",
            manifest.encode().unwrap().compressed,
        )
    };

    // Every row was checked (and the manifest stored); GC collects them all (nothing
    // references them yet) before the snapshot is inserted.
    let pause = hooks::arm(Point::CommitReady, &t.scope);
    let committing = spawn_send(&app, commit());
    pause.reached().await;
    let counts = run_now(&app.state, JobName::Gc).await;
    assert_eq!(counts.get("chunks_deleted"), 1);
    assert_eq!(counts.get("manifests_deleted"), 1);
    pause.release();
    let (status, body) = committing.await.unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "chunk_missing");
    let folder: Uuid = t.folder.parse().unwrap();
    assert!(
        snapshots::Entity::find()
            .filter(snapshots::Column::FolderId.eq(folder))
            .all(&app.state.db)
            .await
            .unwrap()
            .is_empty()
    );
    assert_consistent(&app, &t.scope).await;
    client.push_chunks(&chunks).await;
    app.send(commit()).await.assert(StatusCode::CREATED);
    assert_consistent(&app, &t.scope).await;
}

#[tokio::test]
async fn orphan_sweep_deletes_old_objects_without_rows() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let head = client.sync(&Tree::sample(), None).await;
    // A presigned upload never committed, and an object outside the layout.
    let orphan = put_object(&app, &t.scope, &pseudo_random(7, 10_000)).await;
    let foreign = format!("scopes/{}/notes/readme", t.scope);
    app.state
        .blobs
        .put(&foreign, bytes::Bytes::from_static(b"x"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Too recent for the real grace period.
    let attempt = scheduler::run_leased(&app.state, JobName::OrphanSweep, true, DAY)
        .await
        .unwrap();
    assert_eq!(succeeded(&attempt).get("objects_deleted"), 0);
    assert!(has_object(&app, &t.scope, &orphan).await);

    let counts = run_now(&app.state, JobName::OrphanSweep).await;
    assert_eq!(counts.get("objects_deleted"), 1);
    assert!(counts.get("unrecognized") >= 1);
    assert!(!has_object(&app, &t.scope, &orphan).await);
    assert!(app.state.blobs.head(&foreign).await.unwrap().is_some());
    assert_consistent(&app, &t.scope).await;
    assert_eq!(client.download(&head).await, Tree::sample());
    let _ = As::Anon;
}
