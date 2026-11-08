//! The retention job against real histories: head, pinned and unresolved conflicts
//! kept, per-day and total limits, folder overrides, unlimited, the event.

use std::time::Duration;

use axum::http::StatusCode;
use cloudit_core::protocol::JobName;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::json;
use uuid::Uuid;

use super::events::Sse;
use super::jobs::run_now;
use super::scopes::{Team, create_folder, id_of, team};
use super::support::TestApp;
use super::sync_client::{SyncClient, Tree};
use crate::db::entities::{snapshot_chunks, snapshots};

/// Commits `n` versions in a row; their ids, oldest first.
async fn history(app: &TestApp, t: &Team, folder: &str, n: usize) -> Vec<String> {
    let client = SyncClient::new(app, &t.editor, folder);
    let mut ids: Vec<String> = Vec::new();
    for i in 0..n {
        let mut tree = Tree::default();
        tree.file("save.dat", format!("version {i}").into_bytes());
        let id = client.sync(&tree, ids.last().map(String::as_str)).await;
        ids.push(id);
    }
    ids
}

async fn remaining(app: &TestApp, folder: &str) -> Vec<String> {
    let mut ids: Vec<String> = snapshots::Entity::find()
        .filter(snapshots::Column::FolderId.eq(folder.parse::<Uuid>().unwrap()))
        .all(&app.state.db)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id.to_string())
        .collect();
    ids.sort();
    ids
}

/// Moves a snapshot `days` days into the past.
async fn age(app: &TestApp, id: &str, days: i64) {
    let at = crate::time::now() - chrono::TimeDelta::days(days);
    snapshots::Entity::update_many()
        .col_expr(snapshots::Column::CreatedAt, Expr::value(at))
        .filter(snapshots::Column::Id.eq(id.parse::<Uuid>().unwrap()))
        .exec(&app.state.db)
        .await
        .unwrap();
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[tokio::test]
async fn keeps_head_pinned_and_conflicts_within_the_limits() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let resp = create_folder(
        &app,
        &t.owner,
        &t.scope,
        json!({ "name": "Kept", "retention_max_versions": 2, "retention_max_per_day": 0 }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    let folder = id_of(&resp);
    let ids = history(&app, &t, &folder, 6).await;
    // Pin the oldest; an unresolved conflict on an old base.
    app.call(
        "PATCH",
        &format!("/api/v1/snapshots/{}", ids[0]),
        &t.editor,
        Some(json!({ "pinned": true })),
    )
    .await
    .assert(StatusCode::OK);
    let client = SyncClient::new(&app, &t.editor, &folder);
    let mut tree = Tree::default();
    tree.file("save.dat", b"conflicting".to_vec());
    let (manifest, chunks) = tree.manifest();
    client.push_chunks(&chunks).await;
    let resp = client.commit(&manifest, Some(&ids[2]), "conflict").await;
    resp.assert(StatusCode::CREATED);
    let conflict = resp.body["snapshot"]["id"].as_str().unwrap().to_owned();

    let mut sse = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .unwrap();
    sse.expect("hello").await;
    let counts = run_now(&app.state, JobName::Retention).await;
    assert_eq!(counts.get("snapshots_deleted"), 2);
    // Kept: head (5), pinned (0), conflict, and the 2 newest candidates (4, 3).
    assert_eq!(
        remaining(&app, &folder).await,
        sorted(vec![
            ids[0].clone(),
            ids[3].clone(),
            ids[4].clone(),
            ids[5].clone(),
            conflict.clone()
        ])
    );
    let data = sse.expect("history_changed").await;
    assert_eq!(data["folder_id"], folder.as_str());
    // Their chunk lists went with them; the default folder (2 snapshots) untouched.
    let deleted: Uuid = ids[1].parse().unwrap();
    assert!(
        snapshot_chunks::Entity::find()
            .filter(snapshot_chunks::Column::SnapshotId.eq(deleted))
            .all(&app.state.db)
            .await
            .unwrap()
            .is_empty()
    );
    app.get(&format!("/api/v1/snapshots/{}", ids[1]), &t.viewer)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.post(
        &format!("/api/v1/snapshots/{}/restore", ids[1]),
        &t.editor,
        json!({}),
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
    // A second run deletes nothing more.
    assert_eq!(
        run_now(&app.state, JobName::Retention)
            .await
            .get("snapshots_deleted"),
        0
    );
    sse.quiet(Duration::from_millis(200)).await;

    // Resolving the conflict makes it a candidate (the newest one: 3 goes).
    app.post(
        &format!("/api/v1/snapshots/{conflict}/resolve"),
        &t.editor,
        json!({}),
    )
    .await
    .assert(StatusCode::OK);
    run_now(&app.state, JobName::Retention).await;
    assert_eq!(
        remaining(&app, &folder).await,
        sorted(vec![ids[0].clone(), ids[4].clone(), ids[5].clone(), conflict])
    );
}

#[tokio::test]
async fn per_day_limit_uses_utc_days_and_server_defaults() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    // Server defaults: 20 versions, 5 per day → 2 per day here.
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &t.site_admin,
        Some(json!({ "retention_max_per_day": 2 })),
    )
    .await
    .assert(StatusCode::OK);
    let ids = history(&app, &t, &t.folder, 7).await;
    // Days: [0, 1, 2] three days ago, [3, 4] yesterday, [5, 6 (head)] today.
    for (i, days) in [(0, 3), (1, 3), (2, 3), (3, 1), (4, 1)] {
        age(&app, &ids[i], days).await;
    }
    run_now(&app.state, JobName::Retention).await;
    // Three days ago: keep the 2 newest (1, 2); yesterday both; today 5 + head.
    assert_eq!(remaining(&app, &t.folder).await, sorted(ids[1..].to_vec()));

    // Unlimited for this folder: nothing more goes, even with a tiny default.
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &t.site_admin,
        Some(json!({ "retention_max_versions": 1, "retention_max_per_day": 1 })),
    )
    .await
    .assert(StatusCode::OK);
    app.call(
        "PATCH",
        &format!("/api/v1/folders/{}", t.folder),
        &t.owner,
        Some(json!({ "retention_max_versions": 0, "retention_max_per_day": 0 })),
    )
    .await
    .assert(StatusCode::OK);
    run_now(&app.state, JobName::Retention).await;
    assert_eq!(remaining(&app, &t.folder).await.len(), 6);
    // Back to the default (1 / 1): only the head and the newest candidate remain.
    app.call(
        "PATCH",
        &format!("/api/v1/folders/{}", t.folder),
        &t.owner,
        Some(json!({ "retention_max_versions": null, "retention_max_per_day": null })),
    )
    .await
    .assert(StatusCode::OK);
    run_now(&app.state, JobName::Retention).await;
    assert_eq!(
        remaining(&app, &t.folder).await,
        sorted(vec![ids[5].clone(), ids[6].clone()])
    );
}
