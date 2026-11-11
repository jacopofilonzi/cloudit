//! Chunk packs (API level 2): uploads stored exactly like single uploads, all-or-nothing
//! verification, quota, limits, roles; pack downloads, missing hashes, the size budget.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use cloudit_core::chunk::codec::{decode_chunk, encode_chunk};
use cloudit_core::hash::ChunkHash;
use cloudit_core::pack::{
    MAX_PACK_BYTES, MAX_PACK_RECORDS, PACK_CONTENT_TYPE, PackWriter, decode_pack, record_header,
};
use serde_json::json;
use uuid::Uuid;

use super::scopes::team;
use super::support::{As, RawResp, Resp, TestApp};
use super::sync_client::{SyncClient, Tree, consistent, pseudo_random, usage_of};
use crate::sync::{chunks, keys};

fn pack_of(chunks: &BTreeMap<ChunkHash, Vec<u8>>) -> Vec<u8> {
    let mut writer = PackWriter::new();
    for (hash, data) in chunks {
        writer.push(hash, &encode_chunk(data).unwrap()).unwrap();
    }
    writer.finish()
}

fn small_chunks(n: u64, len: usize) -> BTreeMap<ChunkHash, Vec<u8>> {
    (0..n)
        .map(|i| {
            let data = pseudo_random(1000 + i, len);
            (ChunkHash::of(&data), data)
        })
        .collect()
}

async fn upload(app: &TestApp, folder: &str, who: &As, body: Vec<u8>) -> Resp {
    app.send(app.raw_request(
        "POST",
        &format!("/api/v1/folders/{folder}/chunks/pack"),
        who,
        PACK_CONTENT_TYPE,
        body,
    ))
    .await
}

async fn download(app: &TestApp, folder: &str, who: &As, hashes: &[ChunkHash]) -> RawResp {
    let hashes: Vec<String> = hashes.iter().map(ToString::to_string).collect();
    app.send_for_bytes(app.request(
        "POST",
        &format!("/api/v1/folders/{folder}/chunks/pack/download"),
        who,
        Some(&json!({ "hashes": hashes })),
    ))
    .await
}

async fn object_exists(app: &TestApp, scope: &str, hash: &ChunkHash) -> bool {
    let scope: Uuid = scope.parse().unwrap();
    app.state
        .blobs
        .head(&keys::chunk(scope, hash))
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn a_pack_upload_stores_like_single_uploads() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let chunks = small_chunks(40, 3000);
    let resp = upload(&app, &t.folder, &t.device, pack_of(&chunks)).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body, json!({ "chunks": 40, "stored": 40 }));

    // Objects and rows, usage = the encoded sizes once each (as single PUTs).
    let encoded: u64 = chunks
        .values()
        .map(|d| encode_chunk(d).unwrap().len() as u64)
        .sum();
    assert_eq!(consistent(usage_of(&app, &t.scope).await), encoded);
    let scope: Uuid = t.scope.parse().unwrap();
    let hashes: Vec<ChunkHash> = chunks.keys().copied().collect();
    let sizes = chunks::sizes(&app.state.db, scope, &hashes).await.unwrap();
    for (hash, data) in &chunks {
        assert_eq!(sizes[hash] as usize, data.len());
        assert!(object_exists(&app, &t.scope, hash).await);
    }
    let client = SyncClient::new(&app, &t.device, &t.folder);
    assert_eq!(client.push_chunks(&chunks).await, 0, "nothing missing");

    // Idempotent, also mixed with new chunks.
    let resp = upload(&app, &t.folder, &t.editor, pack_of(&chunks)).await;
    assert_eq!(resp.body, json!({ "chunks": 40, "stored": 0 }));
    let mut more = small_chunks(42, 3000);
    more.retain(|h, _| !chunks.contains_key(h));
    more.extend(chunks.iter().take(3).map(|(h, d)| (*h, d.clone())));
    let resp = upload(&app, &t.folder, &t.editor, pack_of(&more)).await;
    assert_eq!(resp.body, json!({ "chunks": 5, "stored": 2 }));
    let encoded_more: u64 = more
        .iter()
        .filter(|(h, _)| !chunks.contains_key(h))
        .map(|(_, d)| encode_chunk(d).unwrap().len() as u64)
        .sum();
    assert_eq!(
        consistent(usage_of(&app, &t.scope).await),
        encoded + encoded_more
    );

    // A commit can use the packed chunks.
    let mut tree = Tree::default();
    for (i, data) in chunks.values().enumerate() {
        tree.file(&format!("f{i:02}"), data.clone());
    }
    client.sync(&tree, None).await;
    // An empty pack is a no-op.
    upload(&app, &t.folder, &t.editor, vec![1])
        .await
        .assert(StatusCode::OK);
}

#[tokio::test]
async fn an_invalid_record_stores_nothing() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let chunks = small_chunks(5, 2000);
    let wrong = ChunkHash::of(b"not this content");
    let mut writer = PackWriter::new();
    for (hash, data) in &chunks {
        writer.push(hash, &encode_chunk(data).unwrap()).unwrap();
    }
    writer
        .push(&wrong, &encode_chunk(b"other").unwrap())
        .unwrap();
    let garbage = ChunkHash::of(b"garbage");
    writer.push(&garbage, &[0x07, 1, 2]).unwrap();
    let resp = upload(&app, &t.folder, &t.editor, writer.finish()).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_invalid");
    let mut expected = vec![wrong.to_string(), garbage.to_string()];
    expected.sort();
    let mut listed: Vec<String> = resp.body["hashes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap().to_owned())
        .collect();
    listed.sort();
    assert_eq!(listed, expected);
    assert_eq!(consistent(usage_of(&app, &t.scope).await), 0);
    for hash in chunks.keys() {
        assert!(!object_exists(&app, &t.scope, hash).await);
    }

    // Malformed packs.
    let mut truncated = pack_of(&chunks);
    truncated.pop();
    let mut duplicate = vec![1];
    for _ in 0..2 {
        let data = encode_chunk(b"dup").unwrap();
        duplicate.extend_from_slice(&record_header(&ChunkHash::of(b"dup"), data.len() as u32));
        duplicate.extend_from_slice(&data);
    }
    let mut many = vec![1];
    for i in 0..=MAX_PACK_RECORDS as u32 {
        let data = encode_chunk(&i.to_le_bytes()).unwrap();
        many.extend_from_slice(&record_header(
            &ChunkHash::of(&i.to_le_bytes()),
            data.len() as u32,
        ));
        many.extend_from_slice(&data);
    }
    for body in [Vec::new(), vec![2], truncated, duplicate, many] {
        upload(&app, &t.folder, &t.editor, body)
            .await
            .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    upload(&app, &t.folder, &t.editor, vec![1; MAX_PACK_BYTES + 1])
        .await
        .assert_error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");
    assert_eq!(consistent(usage_of(&app, &t.scope).await), 0);
}

#[tokio::test]
async fn pack_uploads_respect_the_quota() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let owner = app.user_id(&t.owner).await;
    let chunks = small_chunks(10, 5000);
    let body = pack_of(&chunks);
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{owner}"),
        &t.site_admin,
        Some(json!({ "quota_bytes": 20_000 })),
    )
    .await
    .assert(StatusCode::OK);
    upload(&app, &t.folder, &t.editor, body.clone())
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");
    assert_eq!(consistent(usage_of(&app, &t.scope).await), 0);
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{owner}"),
        &t.site_admin,
        Some(json!({ "quota_bytes": null })),
    )
    .await
    .assert(StatusCode::OK);
    upload(&app, &t.folder, &t.editor, body)
        .await
        .assert(StatusCode::OK);
}

#[tokio::test]
async fn pack_endpoints_follow_the_roles() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let chunks = small_chunks(2, 100);
    let hashes: Vec<ChunkHash> = chunks.keys().copied().collect();
    for who in [&t.owner, &t.device, &t.admin, &t.editor] {
        upload(&app, &t.folder, who, pack_of(&chunks))
            .await
            .assert(StatusCode::OK);
    }
    upload(&app, &t.folder, &t.viewer, pack_of(&chunks))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    download(&app, &t.folder, &t.viewer, &hashes)
        .await
        .assert(StatusCode::OK);
    for who in [&t.outsider, &t.site_admin] {
        upload(&app, &t.folder, who, pack_of(&chunks))
            .await
            .assert_error(StatusCode::NOT_FOUND, "not_found");
        download(&app, &t.folder, who, &hashes)
            .await
            .assert(StatusCode::NOT_FOUND);
    }
    upload(&app, &t.folder, &As::Anon, pack_of(&chunks))
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn pack_downloads_carry_what_the_scope_has() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let chunks = small_chunks(30, 4000);
    upload(&app, &t.folder, &t.editor, pack_of(&chunks))
        .await
        .assert(StatusCode::OK);
    let unknown = ChunkHash::of(b"never uploaded");
    let mut wanted: Vec<ChunkHash> = chunks.keys().copied().collect();
    wanted.push(unknown);
    wanted.push(wanted[0]);
    let resp = download(&app, &t.folder, &t.viewer, &wanted).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.headers["content-type"], PACK_CONTENT_TYPE);
    let entries = decode_pack(&resp.body).unwrap();
    let got: Vec<ChunkHash> = entries.iter().map(|e| e.hash).collect();
    assert_eq!(
        got,
        chunks.keys().copied().collect::<Vec<_>>(),
        "sorted, no unknown"
    );
    for entry in &entries {
        assert_eq!(
            decode_chunk(&resp.body[entry.range.clone()], &entry.hash).unwrap(),
            chunks[&entry.hash]
        );
    }
    // Nothing the scope has: an empty pack.
    let resp = download(&app, &t.folder, &t.viewer, &[unknown]).await;
    assert_eq!(&resp.body[..], &[1]);
    // Too many hashes.
    let many: Vec<ChunkHash> = (0..=MAX_PACK_RECORDS as u32)
        .map(|i| ChunkHash::of(&i.to_le_bytes()))
        .collect();
    let resp = download(&app, &t.folder, &t.viewer, &many).await;
    resp.assert(StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn pack_downloads_stop_at_the_size_limit() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    // 20 incompressible chunks of 1 MiB: more than a pack holds.
    let big: BTreeMap<ChunkHash, Vec<u8>> = (0..20u64)
        .map(|i| {
            let data = pseudo_random(5000 + i, 1024 * 1024);
            (ChunkHash::of(&data), data)
        })
        .collect();
    assert_eq!(client.push_chunks(&big).await, 20);
    let wanted: Vec<ChunkHash> = big.keys().copied().collect();
    let resp = download(&app, &t.folder, &t.viewer, &wanted).await;
    resp.assert(StatusCode::OK);
    assert!(resp.body.len() <= MAX_PACK_BYTES);
    let entries = decode_pack(&resp.body).unwrap();
    assert_eq!(
        entries.len(),
        15,
        "16 MiB hold 15 records of 1 MiB + header"
    );
    let got: Vec<ChunkHash> = entries.iter().map(|e| e.hash).collect();
    assert_eq!(got, wanted[..15]);
}
