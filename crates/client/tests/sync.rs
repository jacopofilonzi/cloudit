//! Push and pull against the real server: first push, incremental push, pull into an
//! empty folder, deletions, empty directories, large files, the executable bit, temp
//! files, ignore patterns, events.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use cloudit_client::events::{RemoteEvent, follow};
use cloudit_client::{CancellationToken, OpContext, SyncOutcome, SyncState};
use common::*;

fn ctx() -> OpContext {
    OpContext::new()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_push_then_incremental_push() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "slot1.sav", noise(300_000, 1));
    write(root, "slot2.sav", noise(300_000, 2));
    write(root, "config/settings.ini", b"volume=7\n");
    let link = new_folder(&a, root, &[]).await;
    assert_eq!(link.emoji.as_deref(), Some("🎮"));

    let status = a.engine.status(link.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::LocalChanges);
    assert_eq!(status.local_changes.len(), 3);

    let report = a.engine.sync(link.id, &ctx()).await.unwrap();
    let SyncOutcome::Pushed {
        uploaded_chunks: first_chunks,
        ..
    } = report.outcome
    else {
        panic!("{report:?}")
    };
    assert!(first_chunks >= 3, "{first_chunks}");
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(
        server.head_paths(&link.folder_id).await,
        vec!["config/settings.ini", "slot1.sav", "slot2.sav"]
    );

    // Nothing changed: nothing to do.
    let again = a.engine.sync(link.id, &ctx()).await.unwrap();
    assert_eq!(again.outcome, SyncOutcome::UpToDate);

    // One small file changed: exactly one chunk goes up.
    write(root, "config/settings.ini", b"volume=9\n");
    let report = a.engine.sync(link.id, &ctx()).await.unwrap();
    match report.outcome {
        SyncOutcome::Pushed {
            uploaded_chunks, ..
        } => assert_eq!(uploaded_chunks, 1),
        other => panic!("{other:?}"),
    }
    // A renamed file costs no upload (its chunks are on the server).
    std::fs::rename(full(root, "slot2.sav"), full(root, "slot3.sav")).unwrap();
    match a.engine.sync(link.id, &ctx()).await.unwrap().outcome {
        SyncOutcome::Pushed {
            uploaded_chunks, ..
        } => assert_eq!(uploaded_chunks, 0),
        other => panic!("{other:?}"),
    }
    let activity = a.engine.activity(Some(link.id), 10).unwrap();
    assert_eq!(activity[0].action, "push");
    assert!(activity.iter().any(|e| e.action == "link"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pull_into_empty_folder_and_deletions() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "a.txt", b"alpha");
    write(da.path(), "deep/nested/b.bin", noise(200_000, 3));
    write(da.path(), "deep/other.txt", b"other");
    std::fs::create_dir_all(full(da.path(), "screenshots")).unwrap();
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    assert!(
        server
            .head_paths(&link_a.folder_id)
            .await
            .contains(&"screenshots".to_owned())
    );

    // B links into a folder that doesn't exist yet: created, then pulled.
    let b_root = db.path().join("Saves");
    let link_b = link_on(&b, &link_a, &b_root, &[]).await;
    assert_eq!(
        b.engine.status(link_b.id, &ctx()).await.unwrap().state,
        SyncState::RemoteChanges
    );
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(read(&b_root, "a.txt"), b"alpha");
    assert_eq!(read(&b_root, "deep/nested/b.bin"), noise(200_000, 3));
    assert!(full(&b_root, "screenshots").is_dir());
    assert_eq!(tree(&b_root), tree(da.path()));
    // Pulled files keep the uploaded mtime.
    let mtime = |root: &std::path::Path| {
        std::fs::metadata(full(root, "a.txt"))
            .unwrap()
            .modified()
            .unwrap()
    };
    assert_eq!(mtime(&b_root), mtime(da.path()));

    // A deletes a file, a whole directory and the empty one; B follows.
    std::fs::remove_file(full(da.path(), "a.txt")).unwrap();
    std::fs::remove_dir_all(full(da.path(), "deep")).unwrap();
    std::fs::remove_dir(full(da.path(), "screenshots")).unwrap();
    write(da.path(), "new.txt", b"new");
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(tree(&b_root), vec!["new.txt"]);
    // Old versions keep the deleted files.
    assert_eq!(server.snapshots(&link_a.folder_id).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_file_round_trip() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // Generated, never committed: ~24 MiB, many chunks.
    let big = noise(24 * 1024 * 1024 + 123, 42);
    write(da.path(), "world/region.dat", &big);
    let link_a = new_folder(&a, da.path(), &[]).await;
    let pushed = a.engine.sync(link_a.id, &ctx()).await.unwrap();
    match pushed.outcome {
        SyncOutcome::Pushed {
            uploaded_chunks,
            uploaded_bytes,
            ..
        } => {
            assert!(uploaded_chunks > 20, "{uploaded_chunks}");
            assert_eq!(uploaded_bytes, big.len() as u64);
        }
        other => panic!("{other:?}"),
    }
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    let pulled = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(pulled.outcome, SyncOutcome::Pulled { downloaded_bytes, .. } if downloaded_bytes == big.len() as u64)
    );
    assert!(read(db.path(), "world/region.dat") == big);

    // A small edit in the middle of the big file: few chunks move, both ways.
    let mut edited = big.clone();
    edited[12 * 1024 * 1024] ^= 0xff;
    write(db.path(), "world/region.dat", &edited);
    match b.engine.sync(link_b.id, &ctx()).await.unwrap().outcome {
        SyncOutcome::Pushed {
            uploaded_chunks, ..
        } => assert!(uploaded_chunks <= 2, "{uploaded_chunks}"),
        other => panic!("{other:?}"),
    }
    match a.engine.sync(link_a.id, &ctx()).await.unwrap().outcome {
        SyncOutcome::Pulled {
            downloaded_bytes, ..
        } => assert!(downloaded_bytes <= 2 * 1024 * 1024, "{downloaded_bytes}"),
        other => panic!("{other:?}"),
    }
    assert!(read(da.path(), "world/region.dat") == edited);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn temp_files_are_never_synced() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", b"a");
    write(dir.path(), ".cloudit-tmp-0123456789abcdef", b"half a file");
    write(
        dir.path(),
        "sub/.cloudit-tmp-fedcba9876543210",
        b"half a file",
    );
    let link = new_folder(&a, dir.path(), &[]).await;
    a.engine.sync(link.id, &ctx()).await.unwrap();
    assert_eq!(
        server.head_paths(&link.folder_id).await,
        vec!["a.txt", "sub"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ignored_files_are_never_uploaded_overwritten_or_deleted() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // A ignores nothing, B ignores `*.log` and `cache/`.
    write(da.path(), "save.dat", b"v1");
    write(da.path(), "debug.log", b"A's log");
    write(da.path(), "cache/blob", b"A's cache");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();

    write(db.path(), "debug.log", b"B's own log");
    write(db.path(), "session.lock", b"lock");
    let link_b = link_on(&b, &link_a, db.path(), &["*.log", "cache/", "session.lock"]).await;
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(read(db.path(), "save.dat"), b"v1");
    assert_eq!(read(db.path(), "debug.log"), b"B's own log");
    assert!(!exists(db.path(), "cache"));

    // B's upload keeps A's ignored entries on the server and adds none of B's.
    write(db.path(), "save.dat", b"v2");
    write(db.path(), "debug.log", b"B's log, longer now");
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(
        server.head_paths(&link_a.folder_id).await,
        vec!["cache/blob", "debug.log", "save.dat"]
    );
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    assert_eq!(read(da.path(), "save.dat"), b"v2");
    assert_eq!(read(da.path(), "debug.log"), b"A's log");

    // A deletes its log: B's stays.
    std::fs::remove_file(full(da.path(), "debug.log")).unwrap();
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(read(db.path(), "debug.log"), b"B's log, longer now");
    assert!(exists(db.path(), "session.lock"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relative_transfer_urls_without_public_url() {
    let server = Server::start_with(&[("PUBLIC_URL", "")]).await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "x.bin", noise(100_000, 9));
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(read(db.path(), "x.bin"), noise(100_000, 9));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn executable_bit_round_trip() {
    use std::os::unix::fs::PermissionsExt as _;
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "run.sh", b"#!/bin/sh\necho hi\n");
    write(da.path(), "data.txt", b"plain");
    std::fs::set_permissions(
        full(da.path(), "run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    let mode = |rel: &str| {
        std::fs::metadata(full(db.path(), rel))
            .unwrap()
            .permissions()
            .mode()
    };
    assert_ne!(mode("run.sh") & 0o111, 0);
    assert_eq!(mode("data.txt") & 0o111, 0);
    // Clearing the bit is a change too.
    std::fs::set_permissions(
        full(db.path(), "run.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let status = b.engine.status(link_b.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::LocalChanges);
}

async fn next(events: &mut tokio::sync::mpsc::Receiver<RemoteEvent>) -> RemoteEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("no event in time")
        .expect("channel closed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_report_remote_head_changes() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();

    let cancel = CancellationToken::new();
    let mut events = follow(&b.engine, cancel.clone());
    assert!(matches!(
        next(&mut events).await,
        RemoteEvent::Connected { .. }
    ));

    write(da.path(), "save.dat", b"v2");
    let pushed = a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let SyncOutcome::Pushed { snapshot_id, .. } = pushed.outcome else {
        panic!("{pushed:?}")
    };
    loop {
        match next(&mut events).await {
            RemoteEvent::HeadChanged {
                folder,
                head_snapshot_id,
                ..
            } => {
                assert_eq!(folder, link_b.id);
                assert_eq!(head_snapshot_id.as_deref(), Some(snapshot_id.as_str()));
                break;
            }
            RemoteEvent::FoldersChanged | RemoteEvent::UsageChanged => {}
            other => panic!("{other:?}"),
        }
    }
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(read(db.path(), "save.dat"), b"v2");

    // Signing out ends the follower.
    b.engine.logout().await.unwrap();
    cancel.cancel();
}
