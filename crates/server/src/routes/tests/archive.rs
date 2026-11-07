//! ZIP download of a snapshot: contents, metadata, access, damaged chunks.
//!
//! The archives are read back with async_zip's in-memory reader: deprecated in 0.0.19
//! in favour of `read1`, but complete and simple for checking contents in tests.
#![allow(deprecated)]

use async_zip::base::read::mem::ZipFileReader;
use axum::http::{StatusCode, header};
use cloudit_core::hash::ChunkHash;
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

use super::scopes::team;
use super::support::TestApp;
use super::sync_client::{Item, SyncClient, Tree};
use crate::sync::keys;

#[tokio::test]
async fn archive_has_the_snapshot_files() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let tree = Tree::sample();
    let id = SyncClient::new(&app, &t.device, &t.folder)
        .sync(&tree, None)
        .await;
    let url = format!("/api/v1/snapshots/{id}/archive");

    for who in [&t.device, &t.viewer] {
        let resp = app.get_bytes(&url, who).await;
        resp.assert(StatusCode::OK);
        assert_eq!(resp.headers[header::CONTENT_TYPE], "application/zip");
        let disposition = resp.headers[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(
            disposition.starts_with("attachment; filename=\"Saves-")
                && disposition.contains(".zip"),
            "{disposition}"
        );

        let zip = ZipFileReader::new(resp.body.to_vec()).await.unwrap();
        let entries = zip.file().entries();
        let names: Vec<String> = entries
            .iter()
            .map(|e| e.filename().as_str().unwrap().to_owned())
            .collect();
        let expected: Vec<String> = tree
            .0
            .iter()
            .map(|(path, item)| match item {
                Item::Dir { .. } => format!("{path}/"),
                Item::File { .. } => path.clone(),
            })
            .collect();
        assert_eq!(names, expected);
        for (index, (path, item)) in tree.0.iter().enumerate() {
            let entry = &entries[index];
            let date = entry.last_modification_date();
            let mode = entry.unix_permissions().unwrap();
            match item {
                Item::Dir { .. } => {
                    assert!(entry.dir().unwrap(), "{path}");
                    assert_eq!(mode & 0o777, 0o755);
                }
                Item::File {
                    data,
                    mtime,
                    executable,
                } => {
                    let mut reader = zip.reader_with_entry(index).await.unwrap();
                    let mut content = Vec::new();
                    reader.read_to_end_checked(&mut content).await.unwrap();
                    assert!(content == *data, "{path}");
                    let expected = if *executable { 0o755 } else { 0o644 };
                    assert_eq!(mode & 0o777, expected, "{path}");
                    // 1_700_000_000 = 2023-11-14 22:13:20 UTC (DOS: 2 s precision).
                    if mtime.secs == 1_700_000_000 {
                        assert_eq!((date.year(), date.month(), date.day()), (2023, 11, 14));
                        assert_eq!((date.hour(), date.minute(), date.second()), (22, 13, 20));
                    }
                }
            }
        }
    }
    app.get_bytes(&url, &t.outsider)
        .await
        .assert(StatusCode::NOT_FOUND);
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{}", t.folder),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.get_bytes(&url, &t.viewer)
        .await
        .assert(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_damaged_chunk_aborts_the_download() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let tree = Tree::sample();
    let (manifest, _) = tree.manifest();
    let id = SyncClient::new(&app, &t.owner, &t.folder)
        .sync(&tree, None)
        .await;
    let last: ChunkHash = manifest
        .entries
        .iter()
        .flat_map(|e| e.chunks.iter())
        .next_back()
        .unwrap()
        .hash;
    let scope = t.scope.parse().unwrap();
    app.state
        .blobs
        .put(
            &keys::chunk(scope, &last),
            bytes::Bytes::from_static(b"\x00garbage"),
        )
        .await
        .unwrap();

    let request = app.request(
        "GET",
        &format!("/api/v1/snapshots/{id}/archive"),
        &t.viewer,
        None,
    );
    let response = app.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "headers are sent first");
    assert!(
        response.into_body().collect().await.is_err(),
        "the body ends with an error"
    );
}
