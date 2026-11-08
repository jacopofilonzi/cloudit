//! Deleting for good: the trash purge after the retention period, the permanent
//! delete endpoints (scope, folder, empty trash), usage back to zero, objects gone.

use axum::http::StatusCode;
use cloudit_core::protocol::JobName;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::json;
use uuid::Uuid;

use super::events::Sse;
use super::jobs::run_now;
use super::scopes::{create_folder, create_scope, id_of, team};
use super::support::{As, TestApp};
use super::sync_client::{SyncClient, Tree, consistent, usage_of};
use crate::db::entities::{folders, scopes, snapshots};
use crate::jobs::store;

async fn objects_under(app: &TestApp, prefix: &str) -> usize {
    use futures::StreamExt as _;
    app.state.blobs.list(prefix).unwrap().count().await
}

async fn used(app: &TestApp, who: &As) -> u64 {
    let resp = app.get("/api/v1/me/usage", who).await;
    resp.assert(StatusCode::OK);
    resp.body["used_bytes"].as_u64().unwrap()
}

async fn trash_ids(app: &TestApp, who: &As) -> Vec<String> {
    let resp = app.get("/api/v1/trash", who).await;
    resp.assert(StatusCode::OK);
    resp.body
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn scope_exists(app: &TestApp, id: &str) -> bool {
    scopes::Entity::find_by_id(id.parse::<Uuid>().unwrap())
        .one(&app.state.db)
        .await
        .unwrap()
        .is_some()
}

async fn folder_exists(app: &TestApp, id: &str) -> bool {
    folders::Entity::find_by_id(id.parse::<Uuid>().unwrap())
        .one(&app.state.db)
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn items_are_purged_after_the_retention_period() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let head = client.sync(&Tree::sample(), None).await;
    // A second folder with its own data, trashed long ago.
    let other = id_of(&create_folder(&app, &t.owner, &t.scope, json!({ "name": "Old" })).await);
    let mut tree = Tree::default();
    tree.file("own.bin", super::sync_client::pseudo_random(42, 300_000));
    SyncClient::new(&app, &t.editor, &other)
        .sync(&tree, None)
        .await;
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{other}"),
        &t.admin,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    // A fresh trashed folder stays.
    let fresh = id_of(&create_folder(&app, &t.owner, &t.scope, json!({ "name": "Fresh" })).await);
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{fresh}"),
        &t.admin,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    let long_ago = crate::time::now() - chrono::TimeDelta::days(31);
    folders::Entity::update_many()
        .col_expr(folders::Column::DeletedAt, Expr::value(long_ago))
        .filter(folders::Column::Id.eq(other.parse::<Uuid>().unwrap()))
        .exec(&app.state.db)
        .await
        .unwrap();
    let before = consistent(usage_of(&app, &t.scope).await);

    let mut sse = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .unwrap();
    sse.expect("hello").await;
    let counts = run_now(&app.state, JobName::TrashPurge).await;
    assert_eq!(counts.get("folders_deleted"), 1);
    assert!(!folder_exists(&app, &other).await);
    assert!(folder_exists(&app, &fresh).await);
    let data = sse.expect("folder_changed").await;
    assert_eq!(data["folder_id"], other.as_str());
    assert_eq!(data["change"], "deleted");
    assert!(
        snapshots::Entity::find()
            .filter(snapshots::Column::FolderId.eq(other.parse::<Uuid>().unwrap()))
            .all(&app.state.db)
            .await
            .unwrap()
            .is_empty()
    );
    // A GC run was requested; it frees the folder's own data.
    assert!(
        store::get(&app.state.db, JobName::Gc)
            .await
            .unwrap()
            .unwrap()
            .requested_at
            .is_some()
    );
    run_now(&app.state, JobName::Gc).await;
    let after = consistent(usage_of(&app, &t.scope).await);
    assert!(after < before, "{after} < {before}");
    assert_eq!(client.download(&head).await, Tree::sample());
}

#[tokio::test]
async fn deleting_a_scope_for_good() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    SyncClient::new(&app, &t.editor, &t.folder)
        .sync(&Tree::sample(), None)
        .await;
    let prefix = format!("scopes/{}", t.scope);
    assert!(objects_under(&app, &prefix).await > 0);
    assert!(used(&app, &t.owner).await > 0);
    let purge = format!("/api/v1/trash/scopes/{}", t.scope);

    // Only trashed scopes, only their owner, only with a session.
    app.call("DELETE", &purge, &t.owner, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call(
        "DELETE",
        &format!("/api/v1/scopes/{}", t.scope),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    for who in [&t.admin, &t.outsider] {
        app.call("DELETE", &purge, who, None)
            .await
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    app.call("DELETE", &purge, &t.device, None)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    let mut member = Sse::open(
        &app.app,
        app.request("GET", "/api/v1/events", &t.viewer, None),
    )
    .await
    .unwrap();
    member.expect("hello").await;

    app.call("DELETE", &purge, &t.owner, None)
        .await
        .assert(StatusCode::ACCEPTED);
    // Gone from the trash and no longer restorable; a purge run is requested.
    assert!(!trash_ids(&app, &t.owner).await.contains(&t.scope));
    app.post(
        &format!("/api/v1/scopes/{}/restore", t.scope),
        &t.owner,
        json!({}),
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call("DELETE", &purge, &t.owner, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    let job = store::get(&app.state.db, JobName::TrashPurge)
        .await
        .unwrap()
        .unwrap();
    assert!(job.requested_at.is_some());

    let counts = run_now(&app.state, JobName::TrashPurge).await;
    assert_eq!(counts.get("scopes_deleted"), 1);
    assert!(!scope_exists(&app, &t.scope).await);
    assert!(!folder_exists(&app, &t.folder).await);
    assert_eq!(objects_under(&app, &prefix).await, 0);
    assert_eq!(used(&app, &t.owner).await, 0);
    let resp = app.get("/api/v1/me/usage", &t.owner).await;
    assert_eq!(resp.body["scopes"], json!([]));
    let data = member.expect("scope_changed").await;
    assert_eq!(data, json!({ "scope_id": t.scope, "change": "deleted" }));
    // Nothing else broke: GC and the sweep have nothing to do.
    assert_eq!(
        run_now(&app.state, JobName::Gc)
            .await
            .get("objects_deleted"),
        0
    );
    assert_eq!(
        run_now(&app.state, JobName::OrphanSweep)
            .await
            .get("objects_deleted"),
        0
    );
}

#[tokio::test]
async fn deleting_a_folder_for_good_needs_admin() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let folder = format!("/api/v1/folders/{}", t.folder);
    app.call("DELETE", &folder, &t.admin, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    let purge = format!("/api/v1/trash/folders/{}", t.folder);
    for who in [&t.editor, &t.viewer, &t.outsider] {
        app.call("DELETE", &purge, who, None)
            .await
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    app.call("DELETE", &purge, &t.admin, None)
        .await
        .assert(StatusCode::ACCEPTED);
    assert!(!trash_ids(&app, &t.owner).await.contains(&t.folder));
    app.post(&format!("{folder}/restore"), &t.owner, json!({}))
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    run_now(&app.state, JobName::TrashPurge).await;
    assert!(!folder_exists(&app, &t.folder).await);
}

#[tokio::test]
async fn emptying_the_trash() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let mine = id_of(&create_scope(&app, &t.owner, "Mine").await);
    let kept = id_of(&create_folder(&app, &t.owner, &mine, json!({ "name": "Kept" })).await);
    let trashed = id_of(&create_folder(&app, &t.owner, &mine, json!({ "name": "Gone" })).await);
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{trashed}"),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.call(
        "DELETE",
        &format!("/api/v1/scopes/{}", t.scope),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    assert_eq!(trash_ids(&app, &t.owner).await.len(), 2);
    // Another user's trash is not touched.
    let other = app.user("other@example.com").await;
    let theirs = id_of(&create_scope(&app, &other, "Theirs").await);
    app.call("DELETE", &format!("/api/v1/scopes/{theirs}"), &other, None)
        .await
        .assert(StatusCode::NO_CONTENT);

    let resp = app.post("/api/v1/trash/empty", &t.owner, json!({})).await;
    resp.assert(StatusCode::ACCEPTED);
    assert_eq!(resp.body, json!({ "scopes": 1, "folders": 1 }));
    assert!(trash_ids(&app, &t.owner).await.is_empty());
    let resp = app.post("/api/v1/trash/empty", &t.owner, json!({})).await;
    assert_eq!(resp.body, json!({ "scopes": 0, "folders": 0 }));

    run_now(&app.state, JobName::TrashPurge).await;
    assert!(!scope_exists(&app, &t.scope).await);
    assert!(!folder_exists(&app, &trashed).await);
    assert!(folder_exists(&app, &kept).await);
    assert!(scope_exists(&app, &theirs).await);
    assert_eq!(trash_ids(&app, &other).await, vec![theirs]);
}
