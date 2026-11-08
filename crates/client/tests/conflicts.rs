//! Two devices on one folder: conflicts and their decisions (keep local, keep server,
//! decide later), conflicts decided elsewhere, first links over existing content.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use cloudit_client::{ErrorKind, LinkedFolder, OpContext, SyncOutcome, SyncState};
use common::*;
use tempfile::TempDir;

fn ctx() -> OpContext {
    OpContext::new()
}

struct Pair {
    server: Server,
    a: Client,
    b: Client,
    da: TempDir,
    db: TempDir,
    link_a: LinkedFolder,
    link_b: LinkedFolder,
}

/// A and B in sync on `save.dat` = v1 and `notes.txt`.
async fn pair() -> Pair {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    write(da.path(), "notes.txt", b"notes");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(read(db.path(), "save.dat"), b"v1");
    Pair {
        server,
        a,
        b,
        da,
        db,
        link_a,
        link_b,
    }
}

/// A pushes v2; B changes to `local` and syncs: a conflict snapshot is stored.
async fn diverge(p: &Pair, local: &[u8]) -> String {
    write(p.da.path(), "save.dat", b"v2 from desk");
    p.a.engine.sync(p.link_a.id, &ctx()).await.unwrap();
    write(p.db.path(), "save.dat", local);
    let status = p.b.engine.status(p.link_b.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::Conflict);
    assert_eq!(status.conflicting, vec!["save.dat".to_owned()]);
    let report = p.b.engine.sync(p.link_b.id, &ctx()).await.unwrap();
    let SyncOutcome::NeedsDecision { conflict_id } = report.outcome else {
        panic!("{report:?}")
    };
    assert_eq!(report.status.state, SyncState::Conflict);
    assert_eq!(
        report.status.pending_conflict_id.as_deref(),
        Some(conflict_id.as_str())
    );
    // Nothing local was touched; the head didn't move.
    assert_eq!(read(p.db.path(), "save.dat"), local);
    let folder = p.server.folder(&p.link_a.folder_id).await;
    assert_eq!(folder["unresolved_conflicts"], 1);
    // Syncing again doesn't upload another conflict.
    let again = p.b.engine.sync(p.link_b.id, &ctx()).await.unwrap();
    assert_eq!(
        again.outcome,
        SyncOutcome::NeedsDecision {
            conflict_id: conflict_id.clone()
        }
    );
    assert_eq!(
        p.server.folder(&p.link_a.folder_id).await["unresolved_conflicts"],
        1
    );
    conflict_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_local_makes_the_device_version_the_head() {
    let p = pair().await;
    diverge(&p, b"v3 from laptop").await;
    let report =
        p.b.engine
            .keep_local(p.link_b.id, None, &ctx())
            .await
            .unwrap();
    assert_eq!(report.status.state, SyncState::InSync, "{report:?}");
    assert_eq!(report.status.unresolved_conflicts, 0);
    assert!(report.status.pending_conflict_id.is_none());
    assert_eq!(read(p.db.path(), "save.dat"), b"v3 from laptop");
    let head = &p.server.snapshots(&p.link_a.folder_id).await[0];
    assert_eq!(head["kind"], "restore");
    assert_eq!(head["is_head"], true);
    // A gets B's version.
    p.a.engine.sync(p.link_a.id, &ctx()).await.unwrap();
    assert_eq!(read(p.da.path(), "save.dat"), b"v3 from laptop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_server_replaces_local_files_once_they_are_stored() {
    let p = pair().await;
    let conflict = diverge(&p, b"v3 from laptop").await;
    let report =
        p.b.engine
            .keep_server(p.link_b.id, None, &ctx())
            .await
            .unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(read(p.db.path(), "save.dat"), b"v2 from desk");
    // The local version stays on the server (resolved conflict) and only that one.
    let snapshots = p.server.snapshots(&p.link_a.folder_id).await;
    let conflicts: Vec<_> = snapshots
        .iter()
        .filter(|s| s["kind"] == "conflict")
        .collect();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["id"], conflict.as_str());
    assert!(conflicts[0]["conflict_resolved_at"].is_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_server_stores_later_local_edits_first() {
    let p = pair().await;
    diverge(&p, b"v3 from laptop").await;
    // The user keeps playing on the laptop before deciding.
    write(p.db.path(), "save.dat", b"v4 from laptop, later");
    p.b.engine
        .keep_server(p.link_b.id, None, &ctx())
        .await
        .unwrap();
    assert_eq!(read(p.db.path(), "save.dat"), b"v2 from desk");
    // v4 was stored first as a new, open conflict snapshot.
    let snapshots = p.server.snapshots(&p.link_a.folder_id).await;
    let open: Vec<_> = snapshots
        .iter()
        .filter(|s| s["kind"] == "conflict" && s["conflict_resolved_at"].is_null())
        .collect();
    assert_eq!(open.len(), 1);
    let (status, manifest) = p
        .server
        .call(
            "GET",
            &format!(
                "/snapshots/{}/manifest?format=json",
                open[0]["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, 200);
    let entry = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "save.dat")
        .unwrap()
        .clone();
    assert_eq!(entry["size"], "v4 from laptop, later".len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decide_later_pauses_until_a_decision() {
    let p = pair().await;
    diverge(&p, b"v3 from laptop").await;
    let link = p.b.engine.decide_later(p.link_b.id).unwrap();
    assert!(link.paused);
    let err = p.b.engine.sync(p.link_b.id, &ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Paused);
    let status = p.b.engine.status(p.link_b.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::Conflict);
    assert!(status.paused);
    // Deciding later still works and unpauses.
    let report =
        p.b.engine
            .keep_local(p.link_b.id, None, &ctx())
            .await
            .unwrap();
    assert!(!report.status.paused);
    assert_eq!(report.status.state, SyncState::InSync);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conflicts_decided_elsewhere_and_by_other_devices() {
    let p = pair().await;
    let conflict = diverge(&p, b"v3 from laptop").await;
    // A sees the server conflict made by B.
    let status = p.a.engine.status(p.link_a.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::ServerConflict);
    assert_eq!(status.unresolved_conflicts, 1);
    // A keeps the server version (its own) for B's conflict: nothing changes for A.
    let report =
        p.a.engine
            .keep_server(p.link_a.id, Some(&conflict), &ctx())
            .await
            .unwrap();
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(read(p.da.path(), "save.dat"), b"v2 from desk");
    // B notices the decision: its pending conflict is gone, the server's version
    // remains, B's local content is on the server, so B can pull.
    let status = p.b.engine.status(p.link_b.id, &ctx()).await.unwrap();
    assert!(status.pending_conflict_id.is_none());
    assert_eq!(status.state, SyncState::Conflict);
    let report =
        p.b.engine
            .keep_server(p.link_b.id, Some(&conflict), &ctx())
            .await
            .unwrap();
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(read(p.db.path(), "save.dat"), b"v2 from desk");
    // Exactly one conflict snapshot exists: B's local content was already stored.
    let snapshots = p.server.snapshots(&p.link_a.folder_id).await;
    assert_eq!(
        snapshots.iter().filter(|s| s["kind"] == "conflict").count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_change_on_both_sides_is_not_a_conflict() {
    let p = pair().await;
    write(p.da.path(), "save.dat", b"same");
    p.a.engine.sync(p.link_a.id, &ctx()).await.unwrap();
    write(p.db.path(), "save.dat", b"same");
    let report = p.b.engine.sync(p.link_b.id, &ctx()).await.unwrap();
    assert_eq!(report.outcome, SyncOutcome::UpToDate);
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(
        report.status.base_snapshot_id,
        report.status.head_snapshot_id
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_link_over_existing_content() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let da = tempfile::tempdir().unwrap();
    write(da.path(), "save.dat", b"server");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();

    // Identical local content: in sync at once, nothing uploaded or written.
    let same = tempfile::tempdir().unwrap();
    write(same.path(), "save.dat", b"server");
    let link = link_on(&b, &link_a, same.path(), &[]).await;
    let report = b.engine.sync(link.id, &ctx()).await.unwrap();
    assert_eq!(report.outcome, SyncOutcome::UpToDate);
    assert_eq!(report.status.state, SyncState::InSync);
    assert!(report.status.base_snapshot_id.is_some());
    b.engine.unlink(link.id).await.unwrap();
    assert!(exists(same.path(), "save.dat"));

    // Different local content: a conflict, the local files stored and untouched.
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "save.dat", b"local");
    write(other.path(), "extra.dat", b"extra");
    let link = link_on(&b, &link_a, other.path(), &[]).await;
    let report = b.engine.sync(link.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::NeedsDecision { .. }),
        "{report:?}"
    );
    assert_eq!(read(other.path(), "save.dat"), b"local");
    let report = b.engine.keep_server(link.id, None, &ctx()).await.unwrap();
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(read(other.path(), "save.dat"), b"server");
    assert!(!exists(other.path(), "extra.dat"));
}
