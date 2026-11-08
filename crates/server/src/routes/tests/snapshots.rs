//! Commits: first and incremental syncs, unchanged commits, conflicts and their
//! resolution, validation of manifests and chunks.

use axum::http::StatusCode;
use cloudit_core::chunk::codec::encode_chunk;
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, Manifest, ManifestEntry};
use serde_json::json;

use super::scopes::{create_folder, id_of, team};
use super::support::TestApp;
use super::sync_client::{Item, SyncClient, Tree, consistent, pseudo_random, ts, usage_of};

/// `tree` with a few bytes changed in the middle of `saves/slot2.sav`.
pub(super) fn edited(tree: &Tree) -> Tree {
    let mut tree = tree.clone();
    if let Some(Item::File { data, .. }) = tree.0.get_mut("saves/slot2.sav") {
        data[350_000..350_010].copy_from_slice(b"0123456789");
    }
    tree
}

#[tokio::test]
async fn first_and_incremental_commits() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let device = SyncClient::new(&app, &t.device, &t.folder);
    let tree = Tree::sample();
    let (manifest, chunks) = tree.manifest();
    assert_eq!(device.push_chunks(&chunks).await, chunks.len());

    let resp = device.commit(&manifest, None, "normal").await;
    resp.assert(StatusCode::CREATED);
    let first = &resp.body["snapshot"];
    assert_eq!(resp.body["unchanged"], false);
    assert_eq!(first["kind"], "normal");
    assert_eq!(first["is_head"], true);
    assert_eq!(first["parent_id"], json!(null));
    assert_eq!(first["file_count"], 5);
    assert_eq!(first["logical_size"], manifest.logical_size());
    assert_eq!(first["manifest_hash"], manifest.hash().unwrap().to_string());
    assert_eq!(first["author_id"], app.user_id(&t.owner).await.as_str());
    assert!(first["device_id"].is_string());
    assert!(!first["author_name"].as_str().unwrap().is_empty());
    assert_eq!(first["device_name"], "Gaming PC");
    assert_eq!(first["pinned"], false);
    let first_id = first["id"].as_str().unwrap().to_owned();

    let folder = app
        .get(&format!("/api/v1/folders/{}", t.folder), &t.viewer)
        .await;
    assert_eq!(folder.body["head_snapshot_id"], first_id.as_str());
    assert_eq!(folder.body["head"]["snapshot_id"], first_id.as_str());
    assert_eq!(folder.body["head"]["file_count"], 5);
    assert_eq!(folder.body["head"]["logical_size"], manifest.logical_size());
    assert_eq!(folder.body["unresolved_conflicts"], 0);
    let list = app
        .get(&format!("/api/v1/scopes/{}/folders", t.scope), &t.viewer)
        .await;
    assert_eq!(list.body[0]["head"]["snapshot_id"], first_id.as_str());

    // One file edited: only its changed chunks are missing.
    let changed = edited(&tree);
    let (manifest2, chunks2) = changed.manifest();
    let check = device.check(&chunks2).await;
    let missing = check.body["missing"].as_array().unwrap().len();
    assert!((1..=2).contains(&missing), "{missing} missing");
    device.upload(&check, &chunks2).await;
    let resp = device.commit(&manifest2, Some(&first_id), "normal").await;
    resp.assert(StatusCode::CREATED);
    let second = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["snapshot"]["parent_id"], first_id.as_str());

    // The same state again: unchanged, nothing created.
    let resp = device.commit(&manifest2, Some(&second), "normal").await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["unchanged"], true);
    assert_eq!(resp.body["snapshot"]["id"], second.as_str());
    // Only the head counts: the first state on top of the second is a new snapshot.
    let resp = device.commit(&manifest, Some(&second), "normal").await;
    resp.assert(StatusCode::CREATED);
    let third = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    let resp = app
        .get(&format!("/api/v1/snapshots/{second}"), &t.viewer)
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["is_head"], false);
    let resp = app
        .get(&format!("/api/v1/snapshots/{third}"), &t.viewer)
        .await;
    assert_eq!(resp.body["is_head"], true);

    // Usage: every distinct chunk and both manifests, once.
    let mut all = chunks.clone();
    all.extend(chunks2);
    let expected: u64 = all
        .values()
        .map(|d| encode_chunk(d).unwrap().len() as u64)
        .sum::<u64>()
        + manifest.encode().unwrap().compressed.len() as u64
        + manifest2.encode().unwrap().compressed.len() as u64;
    assert_eq!(consistent(usage_of(&app, &t.scope).await), expected);
}

#[tokio::test]
async fn racing_devices_conflict_and_resolve() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let a = SyncClient::new(&app, &t.device, &t.folder);
    let b = SyncClient::new(&app, &t.editor, &t.folder);
    let base = a.sync(&Tree::sample(), None).await;

    let mut tree_a = Tree::sample();
    tree_a.file("a.txt", b"from a".to_vec());
    let mut tree_b = Tree::sample();
    tree_b.file("b.txt", b"from b".to_vec());
    let (manifest_b, chunks_b) = tree_b.manifest();
    b.push_chunks(&chunks_b).await;
    let won = a.sync(&tree_a, Some(&base)).await;

    // B lost the race: 409 with the current head; nothing was created.
    let resp = b.commit(&manifest_b, Some(&base), "normal").await;
    resp.assert_error(StatusCode::CONFLICT, "conflict");
    assert_eq!(resp.body["head_snapshot_id"], won.as_str());
    // Without a parent while the folder has a head: conflict too.
    b.commit(&manifest_b, None, "normal")
        .await
        .assert_error(StatusCode::CONFLICT, "conflict");

    // B keeps its state as a conflict snapshot: the head doesn't move.
    let resp = b.commit(&manifest_b, Some(&base), "conflict").await;
    resp.assert(StatusCode::CREATED);
    let conflict = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["snapshot"]["kind"], "conflict");
    assert_eq!(resp.body["snapshot"]["is_head"], false);
    assert_eq!(resp.body["snapshot"]["parent_id"], base.as_str());
    assert_eq!(resp.body["snapshot"]["conflict_resolved_at"], json!(null));
    let folder = format!("/api/v1/folders/{}", t.folder);
    let resp = app.get(&folder, &t.viewer).await;
    assert_eq!(resp.body["head_snapshot_id"], won.as_str());
    assert_eq!(resp.body["unresolved_conflicts"], 1);

    // Keep the server version.
    let resolve = format!("/api/v1/snapshots/{conflict}/resolve");
    app.post(&resolve, &t.viewer, json!({}))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    let resp = app.post(&resolve, &t.editor, json!({})).await;
    resp.assert(StatusCode::OK);
    assert!(resp.body["conflict_resolved_at"].is_string());
    let again = app.post(&resolve, &t.device, json!({})).await;
    again.assert(StatusCode::OK);
    assert_eq!(
        again.body["conflict_resolved_at"],
        resp.body["conflict_resolved_at"]
    );
    assert_eq!(
        app.get(&folder, &t.viewer).await.body["unresolved_conflicts"],
        0
    );
    app.post(
        &format!("/api/v1/snapshots/{won}/resolve"),
        &t.editor,
        json!({}),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(
        app.get(&folder, &t.viewer).await.body["head_snapshot_id"],
        won.as_str()
    );
}

#[tokio::test]
async fn concurrent_commits_have_one_winner() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let clients: Vec<SyncClient<'_>> = [&t.owner, &t.device, &t.editor, &t.admin]
        .into_iter()
        .map(|w| SyncClient::new(&app, w, &t.folder))
        .collect();
    let base = clients[0].sync(&Tree::sample(), None).await;
    let mut manifests = Vec::new();
    for (i, client) in clients.iter().enumerate() {
        let mut tree = Tree::sample();
        tree.file("who.txt", format!("client {i}").into_bytes());
        let (manifest, chunks) = tree.manifest();
        client.push_chunks(&chunks).await;
        manifests.push(manifest);
    }
    let commits = clients
        .iter()
        .zip(&manifests)
        .map(|(c, m)| c.commit(m, Some(&base), "normal"));
    let results = futures::future::join_all(commits).await;
    let won: Vec<_> = results
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .collect();
    assert_eq!(won.len(), 1, "{results:?}");
    let head = won[0].body["snapshot"]["id"].as_str().unwrap();
    for lost in results.iter().filter(|r| r.status != StatusCode::CREATED) {
        lost.assert_error(StatusCode::CONFLICT, "conflict");
        assert_eq!(lost.body["head_snapshot_id"], head);
    }
}

/// Stored manifest bytes of arbitrary JSON.
fn stored(json: &str) -> Vec<u8> {
    zstd::bulk::compress(json.as_bytes(), 3).unwrap()
}

#[tokio::test]
async fn commits_are_validated() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let data = pseudo_random(3, 200_000);
    let hash = ChunkHash::of(&data);
    let entry = |path: &str, size: u32| {
        ManifestEntry::file(
            path,
            ts(1_700_000_000, 0),
            false,
            vec![ChunkRef { hash, size }],
        )
    };
    let manifest = Manifest::new(vec![entry("a.bin", 200_000)]).unwrap();

    // The chunk isn't uploaded yet.
    let resp = client.commit(&manifest, None, "normal").await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_missing");
    assert_eq!(resp.body["hashes"], json!([hash.to_string()]));
    app.send_raw(
        "PUT",
        &client.url(&format!("/chunks/{hash}")),
        &t.editor,
        encode_chunk(&data).unwrap(),
    )
    .await
    .assert(StatusCode::NO_CONTENT);

    // Invalid manifests: each answers its core code.
    let canonical = String::from_utf8(manifest.canonical_json().unwrap()).unwrap();
    let pretty = serde_json::to_string_pretty(&manifest).unwrap();
    let unsorted = Manifest {
        version: 1,
        entries: vec![entry("b.bin", 200_000), entry("a.bin", 200_000)],
    };
    let unsorted = String::from_utf8(unsorted.canonical_json().unwrap()).unwrap();
    let cases = [
        (b"not zstd".to_vec(), "manifest_corrupt"),
        (stored("{\"version\":1"), "manifest_invalid_json"),
        (stored(&pretty), "manifest_non_canonical"),
        (
            stored(&canonical.replace("\"version\":1", "\"version\":2")),
            "manifest_unsupported_version",
        ),
        (stored(&unsorted), "manifest_not_sorted"),
        (
            stored(&canonical.replace("a.bin", "../a.bin")),
            "manifest_invalid_path",
        ),
    ];
    for (body, detail) in cases {
        let resp = client.commit_raw(body, None, "normal").await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "manifest_invalid");
        assert_eq!(resp.body["detail"], detail, "{}", resp.body);
    }
    // A chunk size that disagrees with the uploaded chunk.
    let wrong = Manifest::new(vec![entry("a.bin", 100)]).unwrap();
    let resp = client.commit(&wrong, None, "normal").await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "manifest_invalid");
    assert_eq!(resp.body["detail"], "manifest_chunk_size_wrong");

    // Parameters.
    let other = id_of(&create_folder(&app, &t.owner, &t.scope, json!({ "name": "Other" })).await);
    let elsewhere = SyncClient::new(&app, &t.editor, &other)
        .commit(&manifest, None, "normal")
        .await;
    elsewhere.assert(StatusCode::CREATED);
    let foreign = elsewhere.body["snapshot"]["id"].as_str().unwrap();
    for parent in [foreign, "x"] {
        client
            .commit(&manifest, Some(parent), "normal")
            .await
            .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
    // An unknown snapshot id (e.g. a base deleted by retention) can't be the head.
    client
        .commit(
            &manifest,
            Some("0190d5a0-0000-7000-8000-000000000000"),
            "normal",
        )
        .await
        .assert_error(StatusCode::CONFLICT, "conflict");
    client
        .commit(&manifest, None, "restore")
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    client
        .commit(&manifest, None, "other")
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    app.send_raw("POST", &client.url("/snapshots"), &t.editor, vec![1])
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    client
        .commit_raw(vec![0; 64 * 1024 * 1024 + 1], None, "normal")
        .await
        .assert_error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");

    // Roles: viewers can't commit; outsiders don't see the folder.
    SyncClient::new(&app, &t.viewer, &t.folder)
        .commit(&manifest, None, "normal")
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    SyncClient::new(&app, &t.outsider, &t.folder)
        .commit(&manifest, None, "normal")
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.get("/api/v1/snapshots/x", &t.viewer)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.get(&format!("/api/v1/snapshots/{foreign}"), &t.outsider)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    // Finally valid.
    client
        .commit(&manifest, None, "normal")
        .await
        .assert(StatusCode::CREATED);
    consistent(usage_of(&app, &t.scope).await);
}
