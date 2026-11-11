//! Storage usage and quotas: estimates and exact charges, shared scopes charged to the
//! owner, server default vs per-user quotas, unlimited, usage in the admin list.

use axum::http::StatusCode;
use cloudit_core::chunk::codec::encode_chunk;
use cloudit_core::hash::ChunkHash;
use serde_json::{Value, json};

use super::scopes::team;
use super::support::{As, TestApp};
use super::sync_client::{SyncClient, Tree, consistent, pseudo_random, usage_of};

async fn set_quota(app: &TestApp, admin: &As, user: &str, quota: Value) {
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{user}"),
        admin,
        Some(json!({ "quota_bytes": quota })),
    )
    .await
    .assert(StatusCode::OK);
}

async fn patch_user(app: &TestApp, admin: &As, user: &str, body: Value) -> super::support::Resp {
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{user}"),
        admin,
        Some(body),
    )
    .await
}

async fn used(app: &TestApp, who: &As) -> u64 {
    let resp = app.get("/api/v1/me/usage", who).await;
    resp.assert(StatusCode::OK);
    resp.body["used_bytes"].as_u64().unwrap()
}

#[tokio::test]
async fn quotas_are_estimated_then_enforced_exactly() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let owner = app.user_id(&t.owner).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    let first = client.sync(&Tree::sample(), None).await;
    let base = consistent(usage_of(&app, &t.scope).await);
    assert_eq!(used(&app, &t.owner).await, base);
    assert_eq!(used(&app, &t.editor).await, 0, "charged to the owner");

    // Incompressible data is stored as size + 1 (raw header): the estimate (size)
    // fits exactly, the real charge doesn't.
    let data = pseudo_random(11, 100_000);
    let hash = ChunkHash::of(&data);
    set_quota(&app, &t.site_admin, &owner, json!(base + 100_000)).await;
    let check = json!({ "chunks": [{ "hash": hash.to_string(), "size": data.len() }] });
    let check_url = client.url("/chunks/check");
    app.post(&check_url, &t.editor, check.clone())
        .await
        .assert(StatusCode::OK);
    let url = client.url(&format!("/chunks/{hash}"));
    app.send_raw("PUT", &url, &t.editor, encode_chunk(&data).unwrap())
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");
    assert_eq!(consistent(usage_of(&app, &t.scope).await), base);
    assert!(
        app.state
            .blobs
            .head(&crate::sync::keys::chunk(t.scope.parse().unwrap(), &hash))
            .await
            .unwrap()
            .is_none(),
        "nothing stored"
    );
    // One byte less of quota: the estimate already fails.
    set_quota(&app, &t.site_admin, &owner, json!(base + 99_999)).await;
    app.post(&check_url, &t.editor, check)
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");

    // Room for the chunk but not for the new manifest: the commit fails.
    let stored = encode_chunk(&data).unwrap().len() as u64;
    set_quota(&app, &t.site_admin, &owner, json!(base + stored)).await;
    app.send_raw("PUT", &url, &t.editor, encode_chunk(&data).unwrap())
        .await
        .assert(StatusCode::NO_CONTENT);
    let mut tree = Tree::sample();
    tree.file("big.bin", data);
    let (manifest, _) = tree.manifest();
    client
        .commit(&manifest, Some(&first), "normal")
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");
    assert_eq!(consistent(usage_of(&app, &t.scope).await), base + stored);

    // Unchanged commits and restores store nothing: allowed at the limit.
    let (same, _) = Tree::sample().manifest();
    client
        .commit(&same, Some(&first), "normal")
        .await
        .assert(StatusCode::OK);
    app.post(
        &format!("/api/v1/snapshots/{first}/restore"),
        &t.editor,
        json!({}),
    )
    .await
    .assert(StatusCode::CREATED);

    // Back to unlimited (no default): the commit goes through.
    set_quota(&app, &t.site_admin, &owner, Value::Null).await;
    let resp = client.commit(&manifest, None, "conflict").await;
    resp.assert(StatusCode::CREATED);
    consistent(usage_of(&app, &t.scope).await);
}

#[tokio::test]
async fn default_and_per_user_quotas() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let owner = app.user_id(&t.owner).await;
    SyncClient::new(&app, &t.owner, &t.folder)
        .sync(&Tree::sample(), None)
        .await;
    let used_now = used(&app, &t.owner).await;

    let me = app.get("/api/v1/me/usage", &t.device).await;
    me.assert(StatusCode::OK);
    assert_eq!(me.body["quota_bytes"], Value::Null, "unlimited by default");
    assert_eq!(me.body["scopes"][0]["id"], t.scope.as_str());
    assert_eq!(me.body["scopes"][0]["name"], "Team");
    assert_eq!(me.body["scopes"][0]["stored_bytes"], used_now);
    assert_eq!(me.body["scopes"][0]["trashed"], false);

    // Server default.
    let settings = app
        .call(
            "PATCH",
            "/api/v1/admin/settings",
            &t.site_admin,
            Some(json!({ "default_quota_bytes": 1000 })),
        )
        .await;
    settings.assert(StatusCode::OK);
    assert_eq!(settings.body["default_quota_bytes"], 1000);
    let me = app.get("/api/v1/me/usage", &t.owner).await;
    assert_eq!(me.body["quota_bytes"], 1000);
    let mut tree = Tree::sample();
    tree.file("more.bin", pseudo_random(5, 5000));
    let (_, chunks) = tree.manifest();
    SyncClient::new(&app, &t.owner, &t.folder)
        .check(&chunks)
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");

    // A per-user quota wins over the default.
    set_quota(&app, &t.site_admin, &owner, json!(1_000_000_000u64)).await;
    let list = app
        .get("/api/v1/admin/users?limit=200", &t.site_admin)
        .await;
    let entry = list.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == owner.as_str())
        .unwrap()
        .clone();
    assert_eq!(entry["used_bytes"], used_now);
    assert_eq!(entry["quota_bytes"], 1_000_000_000u64);
    assert_eq!(entry["effective_quota_bytes"], 1_000_000_000u64);
    let editor = app.user_id(&t.editor).await;
    let other = list.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == editor.as_str())
        .unwrap();
    assert_eq!(other["used_bytes"], 0);
    assert_eq!(other["quota_bytes"], Value::Null);
    assert_eq!(other["effective_quota_bytes"], 1000);
    assert_eq!(
        SyncClient::new(&app, &t.owner, &t.folder)
            .push_chunks(&chunks)
            .await,
        1
    );

    // Clearing: the user falls back to the default, the default to unlimited.
    let resp = app
        .call(
            "PATCH",
            &format!("/api/v1/admin/users/{owner}"),
            &t.site_admin,
            Some(json!({ "quota_bytes": null })),
        )
        .await;
    assert_eq!(resp.body["quota_bytes"], Value::Null);
    assert_eq!(resp.body["effective_quota_bytes"], 1000);
    assert_eq!(resp.body["admin"], false, "other flags unchanged");
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &t.site_admin,
        Some(json!({ "default_quota_bytes": null })),
    )
    .await
    .assert(StatusCode::OK);
    let me = app.get("/api/v1/me/usage", &t.owner).await;
    assert_eq!(me.body["quota_bytes"], Value::Null);

    // Validation and access.
    for body in [json!({ "quota_bytes": -1 }), json!({ "quota_bytes": "x" })] {
        app.call(
            "PATCH",
            &format!("/api/v1/admin/users/{owner}"),
            &t.site_admin,
            Some(body),
        )
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &t.site_admin,
        Some(json!({ "default_quota_bytes": u64::MAX })),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{owner}"),
        &t.owner,
        Some(json!({ "quota_bytes": 1 })),
    )
    .await
    .assert_error(StatusCode::FORBIDDEN, "forbidden");

    // Trashed scopes still count.
    app.call(
        "DELETE",
        &format!("/api/v1/scopes/{}", t.scope),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    let me = app.get("/api/v1/me/usage", &t.owner).await;
    assert_eq!(me.body["scopes"][0]["trashed"], true);
    assert!(me.body["used_bytes"].as_u64().unwrap() > used_now);
}

fn entry(list: &Value, id: &str) -> Value {
    list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap()
        .clone()
}

#[tokio::test]
async fn unlimited_accounts_ignore_the_default() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let owner = app.user_id(&t.owner).await;
    let editor = app.user_id(&t.editor).await;
    let client = SyncClient::new(&app, &t.editor, &t.folder);
    client.sync(&Tree::sample(), None).await;
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &t.site_admin,
        Some(json!({ "default_quota_bytes": 1000 })),
    )
    .await
    .assert(StatusCode::OK);
    let mut tree = Tree::sample();
    tree.file("more.bin", pseudo_random(5, 5000));
    let (_, chunks) = tree.manifest();
    client
        .check(&chunks)
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");

    // Unlimited: no quota in force, the upload beyond the default goes through.
    let resp = patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": true }),
    )
    .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["quota_unlimited"], true);
    assert_eq!(resp.body["quota_bytes"], Value::Null);
    assert_eq!(resp.body["effective_quota_bytes"], Value::Null);
    let me = app.get("/api/v1/me/usage", &t.owner).await;
    assert_eq!(me.body["quota_bytes"], Value::Null, "unlimited");
    let list = app
        .get("/api/v1/admin/users?limit=200", &t.site_admin)
        .await;
    let e = entry(&list.body, &owner);
    assert_eq!(e["quota_unlimited"], true);
    assert_eq!(e["effective_quota_bytes"], Value::Null);
    let other = entry(&list.body, &editor);
    assert_eq!(other["quota_unlimited"], false);
    assert_eq!(
        other["effective_quota_bytes"], 1000,
        "others keep the default"
    );
    assert_eq!(client.push_chunks(&chunks).await, 1);
    consistent(usage_of(&app, &t.scope).await);

    // Both an unlimited flag and a size: refused, nothing changed.
    let resp = patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": true, "quota_bytes": 5000, "admin": true }),
    )
    .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "quota_unlimited");
    let list = app
        .get("/api/v1/admin/users?limit=200", &t.site_admin)
        .await;
    let e = entry(&list.body, &owner);
    assert_eq!(e["quota_unlimited"], true);
    assert_eq!(e["admin"], false, "validated before any write");

    // `true` with `null` is unlimited too; `false` alone goes back to the default.
    let resp = patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": true, "quota_bytes": null }),
    )
    .await;
    assert_eq!(resp.body["quota_unlimited"], true);
    let resp = patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": false }),
    )
    .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["quota_unlimited"], false);
    assert_eq!(resp.body["effective_quota_bytes"], 1000);

    // A custom size clears it; `false` alone then keeps the size.
    patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": true }),
    )
    .await
    .assert(StatusCode::OK);
    let resp = patch_user(&app, &t.site_admin, &owner, json!({ "quota_bytes": 2000 })).await;
    assert_eq!(resp.body["quota_unlimited"], false);
    assert_eq!(resp.body["quota_bytes"], 2000);
    assert_eq!(resp.body["effective_quota_bytes"], 2000);
    let resp = patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": false }),
    )
    .await;
    assert_eq!(resp.body["quota_bytes"], 2000);

    // `null` (server default) clears it too, and the default applies again.
    patch_user(
        &app,
        &t.site_admin,
        &owner,
        json!({ "quota_unlimited": true }),
    )
    .await
    .assert(StatusCode::OK);
    let resp = patch_user(&app, &t.site_admin, &owner, json!({ "quota_bytes": null })).await;
    assert_eq!(resp.body["quota_unlimited"], false);
    assert_eq!(resp.body["quota_bytes"], Value::Null);
    assert_eq!(resp.body["effective_quota_bytes"], 1000);
    let mut tree = Tree::sample();
    tree.file("again.bin", pseudo_random(6, 5000));
    let (_, chunks) = tree.manifest();
    client
        .check(&chunks)
        .await
        .assert_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded");
}
