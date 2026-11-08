//! The safety rule: the engine never overwrites or deletes a local file whose current
//! content isn't stored on the server. Also cancellation, locked files and bad names.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cloudit_client::{ErrorKind, OpContext, Phase, SyncOutcome, SyncState};
use cloudit_server::jobs::scheduler::run_leased;
use common::*;
use serde_json::json;

fn ctx() -> OpContext {
    OpContext::new()
}

fn no_temp_files(root: &std::path::Path) {
    let leftovers: Vec<_> = tree(root)
        .into_iter()
        .filter(|p| p.contains(".cloudit-tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_changed_between_scan_and_apply_is_never_overwritten() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "a.sav", b"a1");
    write(da.path(), "b.sav", b"b1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();

    write(da.path(), "a.sav", b"a2 from desk");
    write(da.path(), "b.sav", b"b2 from desk");
    a.engine.sync(link_a.id, &ctx()).await.unwrap();

    // The game on the laptop saves `b.sav` while the pull is staging.
    let root = db.path().to_path_buf();
    let saved = Arc::new(AtomicBool::new(false));
    let flag = saved.clone();
    let racing = ctx().with_progress(move |p| {
        if matches!(p.phase, Phase::Staging { .. }) && !flag.swap(true, Ordering::SeqCst) {
            write(&root, "b.sav", b"b-game-save, longer than before");
        }
    });
    let err = b.engine.sync(link_b.id, &racing).await.unwrap_err();
    assert!(saved.load(Ordering::SeqCst));
    assert_eq!(err.kind(), ErrorKind::LocalChanged, "{err:?}");
    // The game's save is intact; a.sav (before b.sav in order) was updated.
    assert_eq!(read(db.path(), "b.sav"), b"b-game-save, longer than before");
    assert_eq!(read(db.path(), "a.sav"), b"a2 from desk");
    no_temp_files(db.path());

    // Now both sides changed b.sav: a conflict, never an overwrite.
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::NeedsDecision { .. }),
        "{report:?}"
    );
    assert_eq!(report.status.conflicting, vec!["b.sav".to_owned()]);
    assert_eq!(read(db.path(), "b.sav"), b"b-game-save, longer than before");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_conflict_upload_leaves_local_files_untouched() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    write(da.path(), "other.dat", b"other v1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    write(da.path(), "save.dat", b"v2 from desk");
    write(da.path(), "other.dat", b"other v2");
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    write(db.path(), "save.dat", b"v3 from laptop");
    let SyncOutcome::NeedsDecision { .. } = b.engine.sync(link_b.id, &ctx()).await.unwrap().outcome
    else {
        panic!("expected a conflict")
    };
    // Later edits on the laptop, then the quota is full: they can't be stored.
    write(db.path(), "save.dat", noise(200_000, 77));
    let (status, _) = server
        .call(
            "PATCH",
            "/admin/settings",
            Some(json!({ "default_quota_bytes": 1 })),
        )
        .await;
    assert_eq!(status, 200);
    let err = b
        .engine
        .keep_server(link_b.id, None, &ctx())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::QuotaExceeded, "{err:?}");
    assert_eq!(read(db.path(), "save.dat"), noise(200_000, 77));
    assert_eq!(read(db.path(), "other.dat"), b"other v1");
    no_temp_files(db.path());

    // The same when both sides changed and the sync itself must store the conflict.
    let (dc, dd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let c = client(&server, "Third").await;
    let (status, _) = server
        .call(
            "PATCH",
            "/admin/settings",
            Some(json!({ "default_quota_bytes": null })),
        )
        .await;
    assert_eq!(status, 200);
    write(dc.path(), "x.dat", b"x1");
    let link_c = new_folder_named(&a, dc.path(), "More").await;
    a.engine.sync(link_c.id, &ctx()).await.unwrap();
    let link_d = link_on(&c, &link_c, dd.path(), &[]).await;
    c.engine.sync(link_d.id, &ctx()).await.unwrap();
    write(dc.path(), "x.dat", b"x2");
    a.engine.sync(link_c.id, &ctx()).await.unwrap();
    server
        .call(
            "PATCH",
            "/admin/settings",
            Some(json!({ "default_quota_bytes": 1 })),
        )
        .await;
    write(dd.path(), "x.dat", noise(150_000, 5));
    let err = c.engine.sync(link_d.id, &ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::QuotaExceeded, "{err:?}");
    assert_eq!(read(dd.path(), "x.dat"), noise(150_000, 5));
}

async fn new_folder_named(
    a: &Client,
    path: &std::path::Path,
    name: &str,
) -> cloudit_client::LinkedFolder {
    let scope = a.engine.create_scope(name, &ctx()).await.unwrap();
    a.engine
        .create_and_link(&scope.id, name, None, path, &[], &ctx())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_base_deleted_by_retention_is_stored_again_before_a_pull() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    let base = b
        .engine
        .folder(link_b.id)
        .unwrap()
        .base_snapshot_id
        .unwrap();

    // The desk saves twice; retention keeps one version: B's base is gone.
    let (status, _) = server
        .call(
            "PATCH",
            &format!("/folders/{}", link_a.folder_id),
            Some(json!({ "retention_max_versions": 1, "retention_max_per_day": 1 })),
        )
        .await;
    assert_eq!(status, 200);
    for v in ["v2", "v3"] {
        write(da.path(), "save.dat", v);
        a.engine.sync(link_a.id, &ctx()).await.unwrap();
    }
    run_leased(
        &server.state,
        cloudit_core::protocol::JobName::Retention,
        true,
        Duration::ZERO,
    )
    .await
    .unwrap();
    let (status, _) = server
        .call("GET", &format!("/snapshots/{base}"), None)
        .await;
    assert_eq!(status, 404);

    // B has no local changes, but v1 is no longer on the server: it is stored again
    // (as a resolved conflict snapshot) before being replaced.
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(read(db.path(), "save.dat"), b"v3");
    let snapshots = server.snapshots(&link_a.folder_id).await;
    let copies: Vec<_> = snapshots
        .iter()
        .filter(|s| s["kind"] == "conflict")
        .collect();
    assert_eq!(copies.len(), 1);
    assert!(copies[0]["conflict_resolved_at"].is_string());
    assert_eq!(report.status.state, SyncState::InSync);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_mid_upload_and_mid_download_is_consistent() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for i in 0..4 {
        write(da.path(), &format!("part{i}.bin"), noise(1_500_000, i));
    }
    let link_a = new_folder(&a, da.path(), &[]).await;

    // Cancel after the first chunk went up: no version, then a clean retry.
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
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(server.folder(&link_a.folder_id).await["head_snapshot_id"].is_null());
    let status = a.engine.status(link_a.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::LocalChanges);
    a.engine.sync(link_a.id, &ctx()).await.unwrap();

    // Cancel mid-download: nothing written, no temp file left.
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    let op = cancel_after_first(
        |p| matches!(p, Phase::Downloading { done_chunks, .. } if *done_chunks >= 1),
    );
    let err = b.engine.sync(link_b.id, &op).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(tree(db.path()).is_empty(), "{:?}", tree(db.path()));

    // Cancel mid-apply: some files complete, none partial; the rest follows.
    let op = cancel_after_first(|p| matches!(p, Phase::Applying { done, .. } if *done >= 1));
    let err = b.engine.sync(link_b.id, &op).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    no_temp_files(db.path());
    for name in tree(db.path()) {
        let i: u64 = name
            .trim_start_matches("part")
            .trim_end_matches(".bin")
            .parse()
            .unwrap();
        assert!(
            read(db.path(), &name) == noise(1_500_000, i),
            "{name} is partial"
        );
    }
    let status = b.engine.status(link_b.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::RemoteChanges, "{status:?}");
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert_eq!(tree(db.path()), tree(da.path()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreadable_files_block_the_upload_and_are_listed() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "ok.txt", b"ok");
    write(dir.path(), "locked.sav", b"in use");
    let link = new_folder(&a, dir.path(), &[]).await;
    let locked = full(dir.path(), "locked.sav");
    let Some(guard) = lock_file(&locked) else {
        eprintln!("skipped: can't make a file unreadable here (root?)");
        return;
    };
    let status = a.engine.status(link.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::Error);
    assert_eq!(status.problems.len(), 1);
    assert_eq!(status.problems[0].path, "locked.sav");
    let err = a.engine.sync(link.id, &ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ScanProblems);
    assert!(server.folder(&link.folder_id).await["head_snapshot_id"].is_null());
    drop(guard);
    a.engine.sync(link.id, &ctx()).await.unwrap();
    assert_eq!(
        server.head_paths(&link.folder_id).await,
        vec!["locked.sav", "ok.txt"]
    );
}

/// Makes a file unreadable until the guard drops: an exclusive handle on Windows,
/// mode 000 on Unix (`None` when that doesn't stop reading, e.g. as root).
#[cfg(windows)]
fn lock_file(path: &std::path::Path) -> Option<impl Drop + use<>> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    struct Guard(#[allow(dead_code)] std::fs::File);
    impl Drop for Guard {
        fn drop(&mut self) {}
    }
    Some(Guard(file))
}

#[cfg(unix)]
fn lock_file(path: &std::path::Path) -> Option<impl Drop + use<>> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
    struct Guard(std::path::PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
        }
    }
    let guard = Guard(path.to_path_buf());
    if std::fs::File::open(path).is_ok() {
        return None;
    }
    Some(guard)
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_and_colliding_names_are_reported() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "fine.txt", b"ok");
    write(dir.path(), "what?.txt", b"bad");
    write(dir.path(), "Save/slot", b"1");
    write(dir.path(), "save/slot", b"2");
    let link = new_folder(&a, dir.path(), &[]).await;
    let status = a.engine.status(link.id, &ctx()).await.unwrap();
    assert_eq!(status.state, SyncState::Error);
    let mut problems: Vec<_> = status
        .problems
        .iter()
        .map(|p| (p.path.as_str(), p.kind))
        .collect();
    problems.sort_by(|a, b| a.0.cmp(b.0));
    use cloudit_client::scan::ProblemKind;
    assert_eq!(
        problems,
        vec![
            ("Save", ProblemKind::NameCollision),
            ("save", ProblemKind::NameCollision),
            ("what?.txt", ProblemKind::InvalidName),
        ]
    );
    let err = a.engine.sync(link.id, &ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ScanProblems);
    // Ignoring them makes the folder syncable.
    a.engine
        .set_ignore(link.id, &strings(&["what?.txt", "/save/"]))
        .await
        .unwrap();
    a.engine.sync(link.id, &ctx()).await.unwrap();
    assert_eq!(
        server.head_paths(&link.folder_id).await,
        vec!["Save/slot", "fine.txt"]
    );
}
