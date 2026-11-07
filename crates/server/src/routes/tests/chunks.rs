//! Chunk check, proxy upload and download; verification, limits, roles, usage.

use axum::http::StatusCode;
use cloudit_core::chunk::codec::{decode_chunk, encode_chunk};
use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::MAX_STORED_CHUNK_BYTES;
use serde_json::json;

use super::scopes::{create_folder, id_of, team};
use super::support::{As, TestApp};
use super::sync_client::{SyncClient, Tree, consistent, local_path, pseudo_random, usage_of};

#[tokio::test]
async fn check_upload_and_download_through_the_proxy() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.device, &t.folder);
    let (_, chunks) = Tree::sample().manifest();
    assert!(chunks.len() > 10, "{} chunks", chunks.len());

    let check = client.check(&chunks).await;
    check.assert(StatusCode::OK);
    let missing = check.body["missing"].as_array().unwrap();
    assert_eq!(missing.len(), chunks.len());
    let first = &missing[0];
    assert_eq!(first["hash"], chunks.keys().next().unwrap().to_string());
    assert_eq!(
        first["upload"]["url"],
        format!(
            "/api/v1/folders/{}/chunks/{}",
            t.folder,
            first["hash"].as_str().unwrap()
        )
    );
    assert_eq!(
        first["upload"]["headers"],
        json!({ "content-type": "application/octet-stream" })
    );
    assert_eq!(client.upload(&check, &chunks).await, chunks.len());
    assert_eq!(
        client.push_chunks(&chunks).await,
        0,
        "nothing missing any more"
    );

    // Usage = the encoded sizes, once each.
    let encoded: u64 = chunks
        .values()
        .map(|data| encode_chunk(data).unwrap().len() as u64)
        .sum();
    assert_eq!(consistent(usage_of(&app, &t.scope).await), encoded);

    // Download URLs for every chunk plus an unknown one, then the bytes.
    let unknown = ChunkHash::of(b"never uploaded");
    let mut hashes: Vec<String> = chunks.keys().map(ToString::to_string).collect();
    hashes.push(unknown.to_string());
    hashes.push(hashes[0].clone());
    let resp = app
        .post(
            &client.url("/chunks/download"),
            &t.viewer,
            json!({ "hashes": hashes }),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["missing"], json!([unknown.to_string()]));
    let list = resp.body["chunks"].as_array().unwrap();
    assert_eq!(list.len(), chunks.len());
    for item in list {
        let hash: ChunkHash = item["hash"].as_str().unwrap().parse().unwrap();
        assert_eq!(item["download"]["kind"], "proxy");
        assert_eq!(item["download"]["method"], "GET");
        let raw = app
            .get_bytes(
                local_path(item["download"]["url"].as_str().unwrap()),
                &t.viewer,
            )
            .await;
        raw.assert(StatusCode::OK);
        assert_eq!(raw.headers["content-type"], "application/octet-stream");
        assert_eq!(decode_chunk(&raw.body, &hash).unwrap(), chunks[&hash]);
    }
    app.get_bytes(&client.url(&format!("/chunks/{unknown}")), &t.viewer)
        .await
        .assert(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn proxy_urls_are_absolute_with_a_public_url() {
    let app = TestApp::with(&[("PUBLIC_URL", "https://cloud.example.com")]).await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.owner, &t.folder);
    let mut tree = Tree::default();
    tree.file("a", b"hello".to_vec());
    let (_, chunks) = tree.manifest();
    let check = client.check(&chunks).await;
    check.assert(StatusCode::OK);
    let hash = chunks.keys().next().unwrap();
    assert_eq!(
        check.body["missing"][0]["upload"]["url"],
        format!(
            "https://cloud.example.com/api/v1/folders/{}/chunks/{hash}",
            t.folder
        )
    );
    assert_eq!(client.upload(&check, &chunks).await, 1);
}

#[tokio::test]
async fn uploads_are_verified_and_bounded() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let data = pseudo_random(7, 100_000);
    let hash = ChunkHash::of(&data);
    let url = client.url(&format!("/chunks/{hash}"));
    let other = ChunkHash::of(b"other");

    // Content that doesn't hash to the URL's hash, garbage, an empty body.
    let resp = app
        .send_raw(
            "PUT",
            &client.url(&format!("/chunks/{other}")),
            &t.editor,
            encode_chunk(&data).unwrap(),
        )
        .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_invalid");
    assert_eq!(resp.body["hashes"], json!([other.to_string()]));
    for body in [vec![0x07, 1, 2, 3], vec![0x01, 0xff, 0xff], Vec::new()] {
        app.send_raw("PUT", &url, &t.editor, body)
            .await
            .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_invalid");
    }
    // Over the limit; a malformed hash.
    app.send_raw("PUT", &url, &t.editor, vec![0; MAX_STORED_CHUNK_BYTES + 1])
        .await
        .assert_error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");
    app.send_raw("PUT", &client.url("/chunks/ABC"), &t.editor, vec![0])
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(consistent(usage_of(&app, &t.scope).await), 0);

    // Valid, and idempotent.
    let encoded = encode_chunk(&data).unwrap();
    for _ in 0..2 {
        app.send_raw("PUT", &url, &t.editor, encoded.clone())
            .await
            .assert(StatusCode::NO_CONTENT);
    }
    assert_eq!(
        consistent(usage_of(&app, &t.scope).await),
        encoded.len() as u64
    );

    // Check validation.
    let check = |chunks: serde_json::Value| {
        app.post(
            &client.url("/chunks/check"),
            &t.editor,
            json!({ "chunks": chunks }),
        )
    };
    check(json!([{ "hash": hash.to_string(), "size": 0 }]))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    check(json!([{ "hash": hash.to_string(), "size": 1_048_577 }]))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    check(json!([
        { "hash": other.to_string(), "size": 5 },
        { "hash": other.to_string(), "size": 6 }
    ]))
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    check(json!([{ "hash": "abc", "size": 5 }]))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    let many: Vec<_> = (0..10_001u32)
        .map(|i| json!({ "hash": ChunkHash::of(&i.to_le_bytes()).to_string(), "size": 1 }))
        .collect();
    check(json!(many))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let resp = check(json!([
        { "hash": hash.to_string(), "size": data.len() },
        { "hash": other.to_string(), "size": 5 },
        { "hash": other.to_string(), "size": 5 }
    ]))
    .await;
    resp.assert(StatusCode::OK);
    let missing: Vec<&str> = resp.body["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["hash"].as_str().unwrap())
        .collect();
    assert_eq!(missing, [other.to_string()]);
}

#[tokio::test]
async fn chunk_endpoints_follow_the_roles() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = b"role check".to_vec();
    let hash = ChunkHash::of(&data);
    let encoded = encode_chunk(&data).unwrap();
    let base = format!("/api/v1/folders/{}/chunks", t.folder);
    let check = json!({ "chunks": [{ "hash": hash.to_string(), "size": data.len() }] });
    let download = json!({ "hashes": [hash.to_string()] });

    for (who, can_write) in [
        (&t.owner, true),
        (&t.device, true),
        (&t.admin, true),
        (&t.editor, true),
        (&t.viewer, false),
    ] {
        let write = if can_write {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        let resp = app.post(&format!("{base}/check"), who, check.clone()).await;
        assert_eq!(resp.status, write, "{who:?}: {}", resp.body);
        let resp = app
            .send_raw("PUT", &format!("{base}/{hash}"), who, encoded.clone())
            .await;
        let expected = if can_write {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(resp.status, expected, "{who:?}: {}", resp.body);
        app.post(&format!("{base}/download"), who, download.clone())
            .await
            .assert(StatusCode::OK);
        app.get_bytes(&format!("{base}/{hash}"), who)
            .await
            .assert(StatusCode::OK);
    }
    for who in [&t.outsider, &t.site_admin] {
        app.post(&format!("{base}/check"), who, check.clone())
            .await
            .assert_error(StatusCode::NOT_FOUND, "not_found");
        app.get_bytes(&format!("{base}/{hash}"), who)
            .await
            .assert(StatusCode::NOT_FOUND);
    }
    app.post(&format!("{base}/check"), &As::Anon, check.clone())
        .await
        .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");

    // A folder in the trash: gone for sync.
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{}", t.folder),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.post(&format!("{base}/check"), &t.device, check)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.get_bytes(&format!("{base}/{hash}"), &t.device)
        .await
        .assert(StatusCode::NOT_FOUND);

    // Dedup is per scope: another scope of the same owner lacks the chunk.
    let other = id_of(&super::scopes::create_scope(&app, &t.owner, "Other").await);
    let folder = id_of(&create_folder(&app, &t.owner, &other, json!({ "name": "F" })).await);
    let resp = app
        .post(
            &format!("/api/v1/folders/{folder}/chunks/check"),
            &t.owner,
            json!({ "chunks": [{ "hash": hash.to_string(), "size": data.len() }] }),
        )
        .await;
    assert_eq!(resp.body["missing"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_duplicate_uploads_count_once() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let data = pseudo_random(9, 300_000);
    let hash = ChunkHash::of(&data);
    let encoded = encode_chunk(&data).unwrap();
    let url = format!("/api/v1/folders/{}/chunks/{hash}", t.folder);
    let who = [&t.owner, &t.device, &t.editor, &t.admin];
    let requests = who
        .iter()
        .map(|w| app.send_raw("PUT", &url, w, encoded.clone()));
    for resp in futures::future::join_all(requests).await {
        resp.assert(StatusCode::NO_CONTENT);
    }
    assert_eq!(
        consistent(usage_of(&app, &t.scope).await),
        encoded.len() as u64
    );
}
