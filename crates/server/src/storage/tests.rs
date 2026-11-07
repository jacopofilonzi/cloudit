use std::time::Duration;

use bytes::Bytes;
use futures::TryStreamExt as _;
use url::Url;

use super::BlobStore;
use crate::config::{S3Config, Secret, StorageConfig};

fn fs_store() -> (tempfile::TempDir, BlobStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::from_config(&StorageConfig::Fs {
        dir: dir.path().join("blobs"),
    })
    .unwrap();
    (dir, store)
}

/// Put/get/head/list/delete against any backend, under a unique prefix.
async fn exercise(store: &BlobStore, prefix: &str) {
    let key_a = format!("{prefix}/chunks/aa");
    let key_b = format!("{prefix}/chunks/bb");
    let other = format!("{prefix}x/chunks/cc");

    assert_eq!(store.get_bytes(&key_a).await.unwrap(), None);
    assert_eq!(store.head(&key_a).await.unwrap(), None);

    store
        .put(&key_a, Bytes::from_static(b"hello"))
        .await
        .unwrap();
    store
        .put(&key_b, Bytes::from(vec![7u8; 300_000]))
        .await
        .unwrap();
    store.put(&other, Bytes::from_static(b"x")).await.unwrap();

    assert_eq!(
        store.get_bytes(&key_a).await.unwrap().as_deref(),
        Some(&b"hello"[..])
    );
    let streamed: Vec<Bytes> = store
        .get(&key_b)
        .await
        .unwrap()
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(streamed.concat(), vec![7u8; 300_000]);

    let meta = store.head(&key_a).await.unwrap().unwrap();
    assert_eq!(meta.key, key_a);
    assert_eq!(meta.size, 5);

    // Overwrite replaces.
    store.put(&key_a, Bytes::from_static(b"bye")).await.unwrap();
    assert_eq!(store.head(&key_a).await.unwrap().unwrap().size, 3);

    // Listing matches whole segments: `{prefix}x/...` is not under `{prefix}`.
    let mut keys: Vec<String> = store
        .list(prefix)
        .unwrap()
        .map_ok(|m| m.key)
        .try_collect()
        .await
        .unwrap();
    keys.sort();
    assert_eq!(keys, vec![key_a.clone(), key_b.clone()]);

    store.delete(&key_a).await.unwrap();
    store.delete(&key_a).await.unwrap(); // idempotent
    assert_eq!(store.head(&key_a).await.unwrap(), None);
    store.delete(&key_b).await.unwrap();
    store.delete(&other).await.unwrap();
    let left: Vec<_> = store.list(prefix).unwrap().try_collect().await.unwrap();
    assert!(left.is_empty());
}

#[tokio::test]
async fn fs_round_trip() {
    let (_dir, store) = fs_store();
    exercise(&store, "owners/test").await;
    store.check().await.unwrap();
}

#[tokio::test]
async fn fs_never_presigns() {
    let (_dir, store) = fs_store();
    assert!(!store.presign_enabled());
    assert_eq!(store.presign_put("a/b").await.unwrap(), None);
    assert_eq!(store.presign_get("a/b").await.unwrap(), None);
}

#[tokio::test]
async fn rejects_invalid_keys() {
    let (_dir, store) = fs_store();
    for key in ["../escape", "a//b", "a/./b", "/abs", "trailing/", ""] {
        assert!(
            store.put(key, Bytes::from_static(b"x")).await.is_err(),
            "{key}"
        );
    }
}

#[tokio::test]
async fn fs_check_fails_when_root_is_gone() {
    let (dir, store) = fs_store();
    std::fs::remove_dir_all(dir.path().join("blobs")).unwrap();
    assert!(store.check().await.is_err());
}

fn s3_config(presign: bool, public_endpoint: Option<&str>) -> S3Config {
    S3Config {
        endpoint: Some(Url::parse("http://minio:9000").unwrap()),
        region: "us-east-1".to_owned(),
        bucket: "cloudit".to_owned(),
        access_key_id: Some("test-key".to_owned()),
        secret_access_key: Some(Secret::new("test-secret")),
        force_path_style: true,
        presign,
        public_endpoint: public_endpoint.map(|u| Url::parse(u).unwrap()),
        presign_ttl: Duration::from_secs(900),
    }
}

#[tokio::test]
async fn s3_presign_disabled_returns_none() {
    let store =
        BlobStore::from_config(&StorageConfig::S3(Box::new(s3_config(false, None)))).unwrap();
    assert_eq!(store.presign_get("a/b").await.unwrap(), None);
}

#[tokio::test]
async fn s3_presign_signs_for_the_public_endpoint() {
    // Signing is local: no S3 server is contacted.
    let config = s3_config(true, Some("https://s3.example.com"));
    let store = BlobStore::from_config(&StorageConfig::S3(Box::new(config))).unwrap();
    let url = store
        .presign_put("owners/o/chunks/abc")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("s3.example.com"));
    assert_eq!(url.path(), "/cloudit/owners/o/chunks/abc");
    let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let get = |k: &str| {
        query
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("X-Amz-Expires"), Some("900"));
    assert!(get("X-Amz-Signature").is_some());
    assert!(get("X-Amz-Credential").unwrap().starts_with("test-key/"));

    // Without a public endpoint the server endpoint is used.
    let store =
        BlobStore::from_config(&StorageConfig::S3(Box::new(s3_config(true, None)))).unwrap();
    let url = store.presign_get("a/b").await.unwrap().unwrap();
    assert_eq!(url.host_str(), Some("minio"));
    assert_eq!(url.port(), Some(9000));
}

/// S3 settings from `TEST_S3_*` (see `make test-ha`), or `None` to skip.
fn live_s3_config() -> Option<S3Config> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let endpoint = var("TEST_S3_ENDPOINT")?;
    Some(S3Config {
        endpoint: Some(Url::parse(&endpoint).unwrap()),
        region: var("TEST_S3_REGION").unwrap_or_else(|| "us-east-1".to_owned()),
        bucket: var("TEST_S3_BUCKET").unwrap_or_else(|| "cloudit-test".to_owned()),
        access_key_id: var("TEST_S3_ACCESS_KEY_ID"),
        secret_access_key: var("TEST_S3_SECRET_ACCESS_KEY").map(Secret::new),
        force_path_style: true,
        presign: true,
        public_endpoint: None,
        presign_ttl: Duration::from_secs(60),
    })
}

#[tokio::test]
async fn s3_live_round_trip_and_presign() {
    let Some(config) = live_s3_config() else {
        eprintln!("skipped: set TEST_S3_ENDPOINT (make test-ha) to run against S3");
        return;
    };
    let store = BlobStore::from_config(&StorageConfig::S3(Box::new(config))).unwrap();
    store.check().await.unwrap();
    let prefix = format!("test/{}", uuid::Uuid::now_v7().simple());
    exercise(&store, &prefix).await;

    // Presigned URLs work with a plain HTTP client and no credentials.
    let key = format!("{prefix}/presigned");
    let client = reqwest::Client::new();
    let put = store.presign_put(&key).await.unwrap().unwrap();
    let response = client.put(put).body("direct").send().await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let get = store.presign_get(&key).await.unwrap().unwrap();
    let body = client.get(get).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(&body[..], b"direct");
    store.delete(&key).await.unwrap();
}
