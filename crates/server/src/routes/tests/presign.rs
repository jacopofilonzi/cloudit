//! Presigned transfers against a real S3 (Garage under `make test-ha`, `TEST_S3_*`):
//! uploads verified at commit, garbage deleted, presigned downloads. Skipped without
//! `TEST_S3_ENDPOINT`.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use cloudit_core::chunk::codec::{decode_chunk, encode_chunk};
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, Manifest, ManifestEntry};
use serde_json::json;

use super::scopes::team;
use super::support::TestApp;
use super::sync_client::{SyncClient, Tree, consistent, pseudo_random, ts, usage_of};
use crate::sync::keys;

/// The server environment for the live S3 of `TEST_S3_*` with presigning on.
fn s3_env() -> Option<Vec<(&'static str, String)>> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let endpoint = var("TEST_S3_ENDPOINT")?;
    let mut env = vec![
        ("STORAGE", "s3".to_owned()),
        ("S3_ENDPOINT", endpoint),
        ("S3_PRESIGN", "true".to_owned()),
        (
            "S3_REGION",
            var("TEST_S3_REGION").unwrap_or_else(|| "us-east-1".to_owned()),
        ),
        (
            "S3_BUCKET",
            var("TEST_S3_BUCKET").unwrap_or_else(|| "cloudit-test".to_owned()),
        ),
    ];
    if let (Some(id), Some(secret)) = (
        var("TEST_S3_ACCESS_KEY_ID"),
        var("TEST_S3_SECRET_ACCESS_KEY"),
    ) {
        env.push(("S3_ACCESS_KEY_ID", id));
        env.push(("S3_SECRET_ACCESS_KEY", secret));
    }
    Some(env)
}

async fn live_app() -> Option<TestApp> {
    let Some(env) = s3_env() else {
        eprintln!("skipped: set TEST_S3_ENDPOINT (make test-ha) to run against S3");
        return None;
    };
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Some(TestApp::with(&pairs).await)
}

/// Uploads every missing chunk of a check answer through its presigned URL.
async fn upload_presigned(
    http: &reqwest::Client,
    check: &serde_json::Value,
    body_of: impl Fn(&ChunkHash) -> Vec<u8>,
) -> usize {
    let missing = check["missing"].as_array().unwrap();
    for item in missing {
        let hash: ChunkHash = item["hash"].as_str().unwrap().parse().unwrap();
        let upload = &item["upload"];
        assert_eq!(upload["kind"], "presigned");
        assert_eq!(upload["method"], "PUT");
        assert_eq!(upload["headers"], json!({}));
        let url = upload["url"].as_str().unwrap();
        assert!(url.starts_with("http"), "{url}");
        let resp = http.put(url).body(body_of(&hash)).send().await.unwrap();
        assert!(resp.status().is_success(), "{}", resp.status());
    }
    missing.len()
}

#[tokio::test]
async fn presigned_uploads_are_verified_at_commit() {
    let Some(app) = live_app().await else {
        return;
    };
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.device, &t.folder);
    let http = reqwest::Client::new();
    let tree = Tree::sample();
    let (manifest, chunks) = tree.manifest();

    let check = client.check(&chunks).await;
    check.assert(StatusCode::OK);
    let uploaded =
        upload_presigned(&http, &check.body, |h| encode_chunk(&chunks[h]).unwrap()).await;
    assert_eq!(uploaded, chunks.len());
    assert_eq!(
        consistent(usage_of(&app, &t.scope).await),
        0,
        "no rows before the commit"
    );
    let resp = client.commit(&manifest, None, "normal").await;
    resp.assert(StatusCode::CREATED);
    let snapshot = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();
    let expected: u64 = chunks
        .values()
        .map(|d| encode_chunk(d).unwrap().len() as u64)
        .sum::<u64>()
        + manifest.encode().unwrap().compressed.len() as u64;
    assert_eq!(consistent(usage_of(&app, &t.scope).await), expected);
    assert_eq!(client.push_chunks(&chunks).await, 0, "rows exist now");

    // Presigned downloads.
    let hashes: Vec<String> = chunks.keys().map(ToString::to_string).collect();
    let resp = app
        .post(
            &client.url("/chunks/download"),
            &t.viewer,
            json!({ "hashes": hashes }),
        )
        .await;
    resp.assert(StatusCode::OK);
    for item in resp.body["chunks"].as_array().unwrap() {
        let hash: ChunkHash = item["hash"].as_str().unwrap().parse().unwrap();
        assert_eq!(item["download"]["kind"], "presigned");
        let url = item["download"]["url"].as_str().unwrap();
        let body = http.get(url).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(decode_chunk(&body, &hash).unwrap(), chunks[&hash]);
    }
    // The manifest and the ZIP go through the server.
    let raw = app
        .get_bytes(&format!("/api/v1/snapshots/{snapshot}/archive"), &t.viewer)
        .await;
    raw.assert(StatusCode::OK);
    assert!(raw.body.len() > 1000);

    // Garbage through a presigned URL: the commit refuses it and deletes the object;
    // a chunk never uploaded is reported missing.
    let good = pseudo_random(21, 150_000);
    let bad = pseudo_random(22, 150_000);
    let never = pseudo_random(23, 1000);
    let mut extra = BTreeMap::new();
    for data in [&good, &bad] {
        extra.insert(ChunkHash::of(data), data.clone());
    }
    let bad_hash = ChunkHash::of(&bad);
    let check = client.check(&extra).await;
    upload_presigned(&http, &check.body, |h| {
        if *h == bad_hash {
            vec![0x00, 1, 2, 3]
        } else {
            encode_chunk(&extra[h]).unwrap()
        }
    })
    .await;
    let entry = |path: &str, data: &[u8]| {
        ManifestEntry::file(
            path,
            ts(1_700_000_000, 0),
            false,
            vec![ChunkRef {
                hash: ChunkHash::of(data),
                size: u32::try_from(data.len()).unwrap(),
            }],
        )
    };
    let with_bad = Manifest::new(vec![entry("good", &good), entry("bad", &bad)]).unwrap();
    let resp = client.commit(&with_bad, Some(&snapshot), "normal").await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_invalid");
    assert_eq!(resp.body["hashes"], json!([bad_hash.to_string()]));
    let scope = t.scope.parse().unwrap();
    assert!(
        app.state
            .blobs
            .head(&keys::chunk(scope, &bad_hash))
            .await
            .unwrap()
            .is_none(),
        "the invalid object is deleted"
    );
    let with_missing = Manifest::new(vec![entry("good", &good), entry("never", &never)]).unwrap();
    let resp = client
        .commit(&with_missing, Some(&snapshot), "normal")
        .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "chunk_missing");
    assert_eq!(
        resp.body["hashes"],
        json!([ChunkHash::of(&never).to_string()])
    );
    // The good chunk was adopted on the way; usage still matches the rows.
    consistent(usage_of(&app, &t.scope).await);
    let check = client.check(&extra).await;
    let missing: Vec<&str> = check.body["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["hash"].as_str().unwrap())
        .collect();
    assert_eq!(missing, [bad_hash.to_string()]);
}
