//! Folders, the trash and retention settings.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde_json::json;

use super::scopes::{create_folder, create_scope, id_of, share, team};
use super::support::TestApp;

fn ts(value: &serde_json::Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn folder_names_are_unique_per_scope() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let alice = app.user("alice@example.com").await;
    let a = id_of(&create_scope(&app, &alice, "A").await);
    let b = id_of(&create_scope(&app, &alice, "B").await);
    let resp = create_folder(&app, &alice, &a, json!({ "name": "Saves" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["scope_id"], a.as_str());
    assert_eq!(resp.body["head_snapshot_id"], json!(null));
    assert_eq!(resp.body["created_by"], app.user_id(&alice).await.as_str());
    create_folder(&app, &alice, &a, json!({ "name": "SAVES" }))
        .await
        .assert_error(StatusCode::CONFLICT, "name_taken");
    create_folder(&app, &alice, &b, json!({ "name": "saves" }))
        .await
        .assert(StatusCode::CREATED);
    let other = id_of(&create_folder(&app, &alice, &a, json!({ "name": "Other" })).await);
    app.call(
        "PATCH",
        &format!("/api/v1/folders/{other}"),
        &alice,
        Some(json!({ "name": "saves" })),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "name_taken");
    let resp = create_folder(&app, &alice, &a, json!({ "name": "\u{7}" })).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let list = app
        .get(&format!("/api/v1/scopes/{a}/folders"), &alice)
        .await;
    let names: Vec<&str> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Other", "Saves"]);
    let resp = app.get(&format!("/api/v1/scopes/{a}"), &alice).await;
    assert_eq!(resp.body["folder_count"], 2);
}

#[tokio::test]
async fn folder_trash_and_restore() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let folder = format!("/api/v1/folders/{}", t.folder);
    app.call("DELETE", &folder, &t.admin, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    // Gone from listings and reads; deleting again: not found.
    app.get(&folder, &t.owner)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call("DELETE", &folder, &t.owner, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    let list = app
        .get(&format!("/api/v1/scopes/{}/folders", t.scope), &t.viewer)
        .await;
    assert_eq!(list.body, json!([]));
    let scope = app
        .get(&format!("/api/v1/scopes/{}", t.scope), &t.viewer)
        .await;
    assert_eq!(scope.body["folder_count"], 0);

    // The trash of the owner and of the admin shows it, with the purge date.
    let admin_id = app.user_id(&t.admin).await;
    for cred in [&t.owner, &t.admin] {
        let resp = app.get("/api/v1/trash", cred).await;
        resp.assert(StatusCode::OK);
        let items = resp.body.as_array().unwrap();
        assert_eq!(items.len(), 1, "{}", resp.body);
        let item = &items[0];
        assert_eq!(item["kind"], "folder");
        assert_eq!(item["id"], t.folder.as_str());
        assert_eq!(item["name"], "Saves");
        assert_eq!(item["scope_id"], t.scope.as_str());
        assert_eq!(item["scope_name"], "Team");
        assert_eq!(item["deleted_by"], admin_id.as_str());
        let days = (ts(&item["purge_at"]) - ts(&item["deleted_at"])).num_days();
        assert_eq!(days, 30);
    }
    for cred in [&t.editor, &t.viewer, &t.outsider, &t.site_admin] {
        assert_eq!(app.get("/api/v1/trash", cred).await.body, json!([]));
    }
    app.get("/api/v1/trash", &t.device)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");

    // The name is free while in the trash; restoring onto it fails, another name works.
    let resp = create_folder(&app, &t.editor, &t.scope, json!({ "name": "saves" })).await;
    resp.assert(StatusCode::CREATED);
    app.post(&format!("{folder}/restore"), &t.admin, json!({}))
        .await
        .assert_error(StatusCode::CONFLICT, "name_taken");
    let resp = app
        .post(
            &format!("{folder}/restore"),
            &t.admin,
            json!({ "name": "  " }),
        )
        .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.post(
        &format!("{folder}/restore"),
        &t.admin,
        json!({ "unknown": 1 }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    let resp = app
        .post(
            &format!("{folder}/restore"),
            &t.admin,
            json!({ "name": "Saves (old)" }),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Saves (old)");
    app.get(&folder, &t.viewer).await.assert(StatusCode::OK);
    assert_eq!(app.get("/api/v1/trash", &t.owner).await.body, json!([]));
}

#[tokio::test]
async fn trashed_scope_hides_its_folders() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let scope = format!("/api/v1/scopes/{}", t.scope);
    let folder = format!("/api/v1/folders/{}", t.folder);
    // One folder trashed before the scope: it stays trashed after the restore.
    let binned = id_of(&create_folder(&app, &t.owner, &t.scope, json!({ "name": "Old" })).await);
    app.call(
        "DELETE",
        &format!("/api/v1/folders/{binned}"),
        &t.owner,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    app.call("DELETE", &scope, &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);

    for (cred, label) in [
        (&t.owner, "owner"),
        (&t.admin, "admin"),
        (&t.device, "device"),
    ] {
        let resp = app.get(&folder, cred).await;
        assert_eq!(resp.status, StatusCode::NOT_FOUND, "{label}");
        let resp = app.get(&format!("{scope}/folders"), cred).await;
        assert_eq!(resp.status, StatusCode::NOT_FOUND, "{label}");
        let resp = app
            .post(&format!("{scope}/folders"), cred, json!({ "name": "New" }))
            .await;
        assert_eq!(resp.status, StatusCode::NOT_FOUND, "{label}");
        let list = app.get("/api/v1/scopes", cred).await;
        assert_eq!(list.body, json!([]), "{label}");
    }
    // The folder of a trashed scope can't be restored on its own.
    app.post(
        &format!("/api/v1/folders/{binned}/restore"),
        &t.owner,
        json!({}),
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
    // Only the owner sees the scope in the trash, and not its folders.
    let resp = app.get("/api/v1/trash", &t.owner).await;
    let items = resp.body.as_array().unwrap();
    assert_eq!(items.len(), 1, "{}", resp.body);
    assert_eq!(items[0]["kind"], "scope");
    assert_eq!(items[0]["id"], t.scope.as_str());
    assert_eq!(items[0]["scope_name"], "Team");
    assert_eq!(app.get("/api/v1/trash", &t.admin).await.body, json!([]));

    // A new scope takes the name; restoring needs another one.
    create_scope(&app, &t.owner, "team")
        .await
        .assert(StatusCode::CREATED);
    app.post(&format!("{scope}/restore"), &t.owner, json!({}))
        .await
        .assert_error(StatusCode::CONFLICT, "name_taken");
    let resp = app
        .post(
            &format!("{scope}/restore"),
            &t.owner,
            json!({ "name": "Team 2" }),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Team 2");
    assert_eq!(resp.body["folder_count"], 1);

    // Back with its members and folders; the other folder is still in the trash.
    app.get(&folder, &t.viewer).await.assert(StatusCode::OK);
    let resp = app.get("/api/v1/trash", &t.owner).await;
    let items = resp.body.as_array().unwrap();
    assert_eq!(items.len(), 1, "{}", resp.body);
    assert_eq!(items[0]["id"], binned.as_str());
    assert_eq!(items[0]["scope_name"], "Team 2");
}

#[tokio::test]
async fn retention_overrides_and_effective_values() {
    let app = TestApp::new().await;
    let site_admin = app.setup_admin().await;
    let owner = app.user("owner@example.com").await;
    let scope = id_of(&create_scope(&app, &owner, "Games").await);

    let resp = create_folder(&app, &owner, &scope, json!({ "name": "Default" })).await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["retention_max_versions"], json!(null));
    assert_eq!(resp.body["retention_max_per_day"], json!(null));
    assert_eq!(resp.body["effective_retention_max_versions"], 20);
    assert_eq!(resp.body["effective_retention_max_per_day"], 5);
    let default = format!("/api/v1/folders/{}", id_of(&resp));

    let resp = create_folder(
        &app,
        &owner,
        &scope,
        json!({ "name": "Custom", "retention_max_versions": 0, "retention_max_per_day": 2 }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["retention_max_versions"], 0);
    assert_eq!(
        resp.body["effective_retention_max_versions"], 0,
        "unlimited"
    );
    assert_eq!(resp.body["effective_retention_max_per_day"], 2);
    let custom = format!("/api/v1/folders/{}", id_of(&resp));

    // New server defaults apply to folders without an override.
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &site_admin,
        Some(json!({ "retention_max_versions": 50, "retention_max_per_day": 1 })),
    )
    .await
    .assert(StatusCode::OK);
    let resp = app.get(&default, &owner).await;
    assert_eq!(resp.body["effective_retention_max_versions"], 50);
    assert_eq!(resp.body["effective_retention_max_per_day"], 1);
    let resp = app.get(&custom, &owner).await;
    assert_eq!(resp.body["effective_retention_max_versions"], 0);
    assert_eq!(resp.body["effective_retention_max_per_day"], 2);

    // PATCH: null resets, omitted keeps, a number sets.
    let resp = app
        .call(
            "PATCH",
            &custom,
            &owner,
            Some(json!({ "retention_max_per_day": null, "name": "Renamed" })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Renamed");
    assert_eq!(resp.body["retention_max_versions"], 0);
    assert_eq!(resp.body["retention_max_per_day"], json!(null));
    assert_eq!(resp.body["effective_retention_max_per_day"], 1);
    for body in [
        json!({ "retention_max_versions": 100_001 }),
        json!({ "retention_max_per_day": 100_001 }),
    ] {
        app.call("PATCH", &custom, &owner, Some(body))
            .await
            .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
    create_folder(
        &app,
        &owner,
        &scope,
        json!({ "name": "Bad", "retention_max_versions": 100_001 }),
    )
    .await
    .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.call(
        "PATCH",
        &custom,
        &owner,
        Some(json!({ "retention_max_per_day": -1 })),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");

    // An editor may rename but not change the retention.
    let editor = app.user("editor@example.com").await;
    share(&app, &owner, &scope, "editor@example.com", "editor").await;
    app.call("PATCH", &custom, &editor, Some(json!({ "name": "Mine" })))
        .await
        .assert(StatusCode::OK);
    app.call(
        "PATCH",
        &custom,
        &editor,
        Some(json!({ "name": "Mine 2", "retention_max_versions": 1 })),
    )
    .await
    .assert_error(StatusCode::FORBIDDEN, "forbidden");
    assert_eq!(app.get(&custom, &owner).await.body["name"], "Mine");
}

#[tokio::test]
async fn folder_emoji() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let folder = format!("/api/v1/folders/{}", t.folder);
    assert_eq!(app.get(&folder, &t.viewer).await.body["emoji"], json!(null));

    // Set on create (trimmed); invalid values are refused.
    let resp = create_folder(
        &app,
        &t.editor,
        &t.scope,
        json!({ "name": "World", "emoji": " 🌍 " }),
    )
    .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["emoji"], "🌍");
    for bad in ["ab", "🌍 🌍", "\u{7}🌍", "12"] {
        let resp = create_folder(
            &app,
            &t.editor,
            &t.scope,
            json!({ "name": "Bad", "emoji": bad }),
        )
        .await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(resp.body["field"], "emoji");
    }

    // Changed by an editor; other fields untouched; "" and null clear it.
    let resp = app
        .call("PATCH", &folder, &t.editor, Some(json!({ "emoji": "🏝️" })))
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["emoji"], "🏝️");
    assert_eq!(resp.body["name"], "Saves");
    let resp = app
        .call(
            "PATCH",
            &folder,
            &t.editor,
            Some(json!({ "name": "Saves 2" })),
        )
        .await;
    assert_eq!(resp.body["emoji"], "🏝️");
    let resp = app
        .call("PATCH", &folder, &t.editor, Some(json!({ "emoji": "" })))
        .await;
    assert_eq!(resp.body["emoji"], json!(null));
    app.call("PATCH", &folder, &t.editor, Some(json!({ "emoji": "🧪" })))
        .await
        .assert(StatusCode::OK);
    let resp = app
        .call("PATCH", &folder, &t.admin, Some(json!({ "emoji": null })))
        .await;
    assert_eq!(resp.body["emoji"], json!(null));
    app.call("PATCH", &folder, &t.editor, Some(json!({ "emoji": "x" })))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");

    // A viewer can't change it.
    app.call("PATCH", &folder, &t.viewer, Some(json!({ "emoji": "🧪" })))
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    assert_eq!(app.get(&folder, &t.viewer).await.body["emoji"], json!(null));
}
