//! What the desktop app's automatic sync relies on: the per-folder settings and the
//! device-wide switch, and `sync_with` without pulls (the program using the folder is
//! running): pushes go through, pulls and conflict decisions are deferred untouched.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use cloudit_client::{AutoSyncSettings, ErrorKind, OpContext, SyncOptions, SyncOutcome, SyncState};
use common::*;

fn ctx() -> OpContext {
    OpContext::new()
}

const NO_PULL: SyncOptions = SyncOptions { allow_pull: false };

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auto_sync_settings_are_validated_and_kept() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    let link = new_folder(&a, dir.path(), &[]).await;
    assert!(!link.auto_sync);
    assert_eq!(link.quiet_secs, 30);
    assert_eq!(link.process_name, None);

    let set = |enabled, quiet_secs, process: Option<&str>| AutoSyncSettings {
        enabled,
        quiet_secs,
        process_name: process.map(str::to_owned),
    };
    let saved = a
        .engine
        .set_auto_sync(link.id, &set(true, 60, Some("  Game.exe ")))
        .unwrap();
    assert!(saved.auto_sync);
    assert_eq!(saved.quiet_secs, 60);
    assert_eq!(saved.process_name.as_deref(), Some("Game.exe"));

    for bad in [
        set(true, 4, None),
        set(true, 3601, None),
        set(true, 30, Some("C:\\Games\\game.exe")),
        set(true, 30, Some("bin/game")),
        set(true, 30, Some("a\u{7}b")),
        set(true, 30, Some(&"x".repeat(256))),
    ] {
        let err = a.engine.set_auto_sync(link.id, &bad).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad:?}");
    }
    let err = a
        .engine
        .set_auto_sync(999, &set(true, 30, None))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotLinked);

    // An empty name clears it; the settings survive a restart.
    a.engine
        .set_auto_sync(link.id, &set(true, 10, Some("  ")))
        .unwrap();
    assert!(a.engine.auto_sync_enabled().unwrap());
    a.engine.set_auto_sync_enabled(false).unwrap();
    let Client { engine, data } = a;
    drop(engine);
    let engine = open_engine(data.path(), None).await;
    let link = engine.folder(link.id).unwrap();
    assert!(link.auto_sync);
    assert_eq!(link.quiet_secs, 10);
    assert_eq!(link.process_name, None);
    assert!(!engine.auto_sync_enabled().unwrap());
    engine.set_auto_sync_enabled(true).unwrap();
    assert!(engine.auto_sync_enabled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn device_settings_survive_restarts_and_the_pause_ends_by_itself() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    assert_eq!(a.engine.paused_until().unwrap(), None);
    assert!(a.engine.notifications_enabled().unwrap());
    assert!(!a.engine.tray_notice_shown().unwrap());

    // Stored with second precision.
    let until = chrono::Utc::now() + chrono::Duration::hours(1);
    let until = chrono::DateTime::from_timestamp(until.timestamp(), 0).unwrap();
    a.engine.set_paused_until(Some(until)).unwrap();
    a.engine.set_notifications_enabled(false).unwrap();
    a.engine.set_tray_notice_shown().unwrap();
    let Client { engine, data } = a;
    drop(engine);
    let engine = open_engine(data.path(), None).await;
    assert_eq!(engine.paused_until().unwrap(), Some(until));
    assert!(!engine.notifications_enabled().unwrap());
    assert!(engine.tray_notice_shown().unwrap());

    // An end in the past is no pause; resuming clears it.
    let past = chrono::Utc::now() - chrono::Duration::minutes(1);
    engine.set_paused_until(Some(past)).unwrap();
    assert_eq!(engine.paused_until().unwrap(), None);
    engine.set_paused_until(Some(until)).unwrap();
    engine.set_paused_until(None).unwrap();
    assert_eq!(engine.paused_until().unwrap(), None);
    engine.set_notifications_enabled(true).unwrap();
    assert!(engine.notifications_enabled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_pulls_local_changes_are_still_pushed() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "save.dat", b"v1");
    let link = new_folder(&a, dir.path(), &[]).await;
    let report = a.engine.sync_with(link.id, NO_PULL, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pushed { .. }),
        "{report:?}"
    );
    assert_eq!(report.status.state, SyncState::InSync);
    assert_eq!(server.head_paths(&link.folder_id).await, vec!["save.dat"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_pulls_remote_changes_are_deferred_untouched() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();
    let base = b.engine.folder(link_b.id).unwrap().base_snapshot_id;

    write(da.path(), "save.dat", b"v2 from desk");
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let report = b
        .engine
        .sync_with(link_b.id, NO_PULL, &ctx())
        .await
        .unwrap();
    assert_eq!(report.outcome, SyncOutcome::Deferred);
    assert_eq!(report.status.state, SyncState::RemoteChanges);
    assert_eq!(read(db.path(), "save.dat"), b"v1");
    assert_eq!(tree(db.path()), vec!["save.dat"]);
    let after = b.engine.folder(link_b.id).unwrap();
    assert_eq!(after.base_snapshot_id, base);
    assert_eq!(after.last_state, Some(SyncState::RemoteChanges));

    // Allowed again (the program exited): the head is applied.
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::Pulled { .. }),
        "{report:?}"
    );
    assert_eq!(read(db.path(), "save.dat"), b"v2 from desk");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_pulls_a_conflict_is_deferred_without_a_snapshot() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let b = client(&server, "Laptop").await;
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(da.path(), "save.dat", b"v1");
    let link_a = new_folder(&a, da.path(), &[]).await;
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    let link_b = link_on(&b, &link_a, db.path(), &[]).await;
    b.engine.sync(link_b.id, &ctx()).await.unwrap();

    write(da.path(), "save.dat", b"v2 from desk");
    a.engine.sync(link_a.id, &ctx()).await.unwrap();
    write(db.path(), "save.dat", b"v2 from laptop");
    let report = b
        .engine
        .sync_with(link_b.id, NO_PULL, &ctx())
        .await
        .unwrap();
    assert_eq!(report.outcome, SyncOutcome::Deferred);
    assert_eq!(report.status.state, SyncState::Conflict);
    assert_eq!(read(db.path(), "save.dat"), b"v2 from laptop");
    let folder = server.folder(&link_a.folder_id).await;
    assert_eq!(folder["unresolved_conflicts"], 0);
    assert_eq!(server.snapshots(&link_a.folder_id).await.len(), 2);
    assert!(
        b.engine
            .folder(link_b.id)
            .unwrap()
            .pending_conflict_id
            .is_none()
    );

    // A normal sync then asks for the decision as usual.
    let report = b.engine.sync(link_b.id, &ctx()).await.unwrap();
    assert!(
        matches!(report.outcome, SyncOutcome::NeedsDecision { .. }),
        "{report:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_folder_refuses_sync_with() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    let link = new_folder(&a, dir.path(), &[]).await;
    a.engine.set_paused(link.id, true).unwrap();
    let err = a
        .engine
        .sync_with(link.id, NO_PULL, &ctx())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Paused);
}
