//! Chunk packs: many small files move in a few requests with a server at API level 2,
//! one request per chunk with an older server; cancellation mid-pack stays consistent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use cloudit_client::{ErrorKind, OpContext, Phase, SyncState};
use common::*;
use serde_json::json;

const FILES: u64 = 300;

fn ctx() -> OpContext {
    OpContext::new()
}

/// Small save files plus one large file (several chunks, some over the pack limit).
fn fill(root: &std::path::Path) {
    for i in 0..FILES {
        write(root, &format!("saves/player{i:03}.json"), noise(1500, i));
    }
    write(root, "world.bin", noise(3_000_000, 999));
}

fn count(requests: &[String], prefix: &str, suffix: &str) -> usize {
    requests
        .iter()
        .filter(|r| r.starts_with(prefix) && r.ends_with(suffix))
        .count()
}

/// Requests moving single chunks (`PUT` / `GET /folders/{id}/chunks/{hash}`).
fn single_transfers(requests: &[String], method: &str) -> usize {
    requests
        .iter()
        .filter(|r| {
            r.starts_with(&format!("{method} /api/v1/folders/"))
                && r.rsplit('/').next().is_some_and(|last| last.len() == 64)
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn small_files_travel_in_packs() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fill(da.path());
    let link_a = new_folder(&a, da.path(), &[]).await;

    server.take_requests();
    let report = a.engine.sync(link_a.id, &ctx()).await.unwrap();
    assert_eq!(report.status.state, SyncState::InSync);
    let pushed = server.take_requests();
    assert_eq!(count(&pushed, "POST ", "/chunks/pack"), 1, "{pushed:#?}");
    let singles = single_transfers(&pushed, "PUT");
    assert!(singles <= 12, "only the large chunks one by one: {singles}");

    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    server.take_requests();
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    let pulled = server.take_requests();
    assert_eq!(
        count(&pulled, "POST ", "/chunks/pack/download"),
        1,
        "{pulled:#?}"
    );
    assert!(single_transfers(&pulled, "GET") <= 12);
    assert_eq!(tree(db.path()), tree(da.path()));
    for i in [0, 150, FILES - 1] {
        let rel = format!("saves/player{i:03}.json");
        assert_eq!(read(db.path(), &rel), noise(1500, i));
    }
    assert_eq!(read(db.path(), "world.bin"), noise(3_000_000, 999));

    // Changing a few small files: one more small pack each way.
    for i in 0..5 {
        write(
            da.path(),
            &format!("saves/player{i:03}.json"),
            noise(1600, 5000 + i),
        );
    }
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let pushed = server.take_requests();
    assert_eq!(count(&pushed, "POST ", "/chunks/pack"), 1);
    assert_eq!(single_transfers(&pushed, "PUT"), 0);
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(tree(db.path()), tree(da.path()));
    assert_eq!(read(db.path(), "saves/player004.json"), noise(1600, 5004));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_level_1_server_gets_single_transfers() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fill(da.path());
    let link_a = new_folder(&a, da.path(), &[]).await;
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    server.set_health(Some(
        json!({ "status": "ok", "version": "0.1.0", "api_level": 1 }),
    ));
    for engine in [&a.engine, &b.engine] {
        engine.check_compatibility(&ctx()).await.unwrap();
    }

    server.take_requests();
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let pushed = server.take_requests();
    assert_eq!(count(&pushed, "POST ", "/chunks/pack"), 0);
    assert!(single_transfers(&pushed, "PUT") >= FILES as usize);
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    let pulled = server.take_requests();
    assert_eq!(count(&pulled, "POST ", "/chunks/pack/download"), 0);
    assert!(single_transfers(&pulled, "GET") >= FILES as usize);
    assert_eq!(tree(db.path()), tree(da.path()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_mid_pack_is_consistent() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // 2500 small files: three packs (1000 records each at most).
    for i in 0..2500u64 {
        write(da.path(), &format!("s/{i:04}.dat"), noise(700, i));
    }
    let link_a = new_folder(&a, da.path(), &[]).await;
    let cancel_after_first = |phase: fn(&Phase) -> bool| {
        let op = ctx();
        let token = op.cancel_token().clone();
        op.with_progress(move |p| {
            if phase(&p.phase) {
                token.cancel();
            }
        })
    };

    let op = cancel_after_first(
        |p| matches!(p, Phase::Uploading { done_chunks, .. } if *done_chunks >= 1),
    );
    let err = a.engine.sync(link_a.id, &op).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled, "{err:?}");
    assert!(server.folder(&link_a.folder_id).await["head_snapshot_id"].is_null());
    a.engine.sync(link_a.id, &ctx()).await.unwrap();

    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    let op = cancel_after_first(
        |p| matches!(p, Phase::Downloading { done_chunks, .. } if *done_chunks >= 1),
    );
    let err = b.engine.sync(link_b.id, &op).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(tree(db.path()).is_empty(), "{:?}", tree(db.path()));
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(tree(db.path()), tree(da.path()));
    assert_eq!(read(db.path(), "s/1234.dat"), noise(700, 1234));
}
