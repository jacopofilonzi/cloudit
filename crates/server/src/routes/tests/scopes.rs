//! Scopes and sharing: the role matrix of every scope and folder endpoint, sharing by
//! email, leaving, name uniqueness.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::support::{As, PASSWORD, Resp, TestApp};

/// A scope `Team` owned by `user@example.com` (who also has a device token), shared
/// with an admin, an editor and a viewer, with one folder `Saves`.
pub(super) struct Team {
    pub owner: As,
    /// The owner's device token.
    pub device: As,
    pub admin: As,
    pub editor: As,
    pub viewer: As,
    /// Signed in, not a member.
    pub outsider: As,
    /// The server admin: no access to other users' scopes.
    pub site_admin: As,
    pub scope: String,
    pub folder: String,
}

pub(super) async fn create_scope(app: &TestApp, who: &As, name: &str) -> Resp {
    app.post("/api/v1/scopes", who, json!({ "name": name }))
        .await
}

pub(super) async fn create_folder(app: &TestApp, who: &As, scope: &str, body: Value) -> Resp {
    app.post(&format!("/api/v1/scopes/{scope}/folders"), who, body)
        .await
}

pub(super) async fn share(app: &TestApp, owner: &As, scope: &str, email: &str, role: &str) {
    app.post(
        &format!("/api/v1/scopes/{scope}/members"),
        owner,
        json!({ "email": email, "role": role }),
    )
    .await
    .assert(StatusCode::CREATED);
}

pub(super) fn id_of(resp: &Resp) -> String {
    resp.body["id"].as_str().unwrap().to_owned()
}

pub(super) async fn team(app: &TestApp) -> Team {
    let site_admin = app.setup_admin().await;
    let (owner, token, _) = super::devices::device(app).await;
    let admin = app.user("sadmin@example.com").await;
    let editor = app.user("editor@example.com").await;
    let viewer = app.user("viewer@example.com").await;
    let outsider = app.user("outsider@example.com").await;
    let resp = create_scope(app, &owner, "Team").await;
    resp.assert(StatusCode::CREATED);
    let scope = id_of(&resp);
    share(app, &owner, &scope, "sadmin@example.com", "admin").await;
    share(app, &owner, &scope, "editor@example.com", "editor").await;
    share(app, &owner, &scope, "viewer@example.com", "viewer").await;
    let resp = create_folder(app, &owner, &scope, json!({ "name": "Saves" })).await;
    resp.assert(StatusCode::CREATED);
    Team {
        owner,
        device: As::Bearer(token),
        admin,
        editor,
        viewer,
        outsider,
        site_admin,
        scope,
        folder: id_of(&resp),
    }
}

impl Team {
    /// Every actor with a label.
    fn actors(&self) -> [(&'static str, &As); 7] {
        [
            ("owner", &self.owner),
            ("device", &self.device),
            ("admin", &self.admin),
            ("editor", &self.editor),
            ("viewer", &self.viewer),
            ("outsider", &self.outsider),
            ("site_admin", &self.site_admin),
        ]
    }
}

#[track_caller]
fn expect(resp: &Resp, who: &str, what: &str, status: StatusCode) {
    assert_eq!(resp.status, status, "{what} as {who}: {}", resp.body);
}

#[tokio::test]
async fn reads_need_membership() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let reads = [
        format!("/api/v1/scopes/{}", t.scope),
        format!("/api/v1/scopes/{}/members", t.scope),
        format!("/api/v1/scopes/{}/folders", t.scope),
        format!("/api/v1/folders/{}", t.folder),
    ];
    for (who, cred) in t.actors() {
        let visible = !matches!(who, "outsider" | "site_admin");
        for uri in &reads {
            let resp = app.get(uri, cred).await;
            if visible {
                expect(&resp, who, uri, StatusCode::OK);
            } else {
                assert_eq!(resp.code(), "not_found", "{uri} as {who}");
                expect(&resp, who, uri, StatusCode::NOT_FOUND);
            }
        }
        let list = app.get("/api/v1/scopes", cred).await;
        expect(&list, who, "list", StatusCode::OK);
        let scopes = list.body.as_array().unwrap();
        assert_eq!(scopes.len(), usize::from(visible), "{who}");
    }
    // The role each member sees, and the owner and folder count.
    for (cred, role) in [
        (&t.owner, "owner"),
        (&t.device, "owner"),
        (&t.admin, "admin"),
        (&t.editor, "editor"),
        (&t.viewer, "viewer"),
    ] {
        let resp = app.get(&format!("/api/v1/scopes/{}", t.scope), cred).await;
        assert_eq!(resp.body["role"], role);
        assert_eq!(resp.body["owner"]["email"], "user@example.com");
        assert_eq!(resp.body["folder_count"], 1);
        assert_eq!(resp.body["name"], "Team");
    }
    // Garbage ids are not found.
    app.get("/api/v1/scopes/not-a-uuid", &t.owner)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.get(
        &format!("/api/v1/folders/{}", uuid::Uuid::now_v7()),
        &t.owner,
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn scope_changes_are_owner_only_and_session_only() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let uri = format!("/api/v1/scopes/{}", t.scope);
    let rename = json!({ "name": "Renamed" });
    for (who, cred) in t.actors() {
        if who == "owner" {
            continue;
        }
        let patch = app.call("PATCH", &uri, cred, Some(rename.clone())).await;
        let delete = app.call("DELETE", &uri, cred, None).await;
        let expected = match who {
            "outsider" | "site_admin" => StatusCode::NOT_FOUND,
            _ => StatusCode::FORBIDDEN,
        };
        expect(&patch, who, "rename", expected);
        expect(&delete, who, "trash", expected);
    }
    let resp = app.call("PATCH", &uri, &t.owner, Some(rename)).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Renamed");
    assert_eq!(resp.body["role"], "owner");
    app.call("DELETE", &uri, &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    // Gone for everyone; only the owner can restore it (with a web session).
    for (who, cred) in t.actors() {
        expect(
            &app.get(&uri, cred).await,
            who,
            "get trashed",
            StatusCode::NOT_FOUND,
        );
        if who == "owner" {
            continue;
        }
        let restore = app.post(&format!("{uri}/restore"), cred, json!({})).await;
        let expected = match who {
            "device" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&restore, who, "restore", expected);
    }
    // Restore without a body.
    let resp = app
        .call("POST", &format!("{uri}/restore"), &t.owner, None)
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "Renamed");
    app.get(&uri, &t.viewer).await.assert(StatusCode::OK);
    // Restoring a scope that isn't in the trash: not found.
    app.call("POST", &format!("{uri}/restore"), &t.owner, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn folder_endpoints_follow_the_roles() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let folders = format!("/api/v1/scopes/{}/folders", t.scope);
    let folder = format!("/api/v1/folders/{}", t.folder);

    // Create: editor and above (device tokens too); a retention override: admin.
    for (who, cred) in t.actors() {
        let resp = app
            .post(&folders, cred, json!({ "name": format!("by {who}") }))
            .await;
        let expected = match who {
            "owner" | "device" | "admin" | "editor" => StatusCode::CREATED,
            "viewer" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "create folder", expected);
        let resp = app
            .post(
                &folders,
                cred,
                json!({ "name": format!("kept by {who}"), "retention_max_versions": 3 }),
            )
            .await;
        let expected = match who {
            "owner" | "device" | "admin" => StatusCode::CREATED,
            "editor" | "viewer" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "create folder with retention", expected);
    }

    // Rename: editor and above, web session only; retention: admin and above.
    for (who, cred) in t.actors() {
        let resp = app
            .call(
                "PATCH",
                &folder,
                cred,
                Some(json!({ "name": format!("Saves {who}") })),
            )
            .await;
        let expected = match who {
            "owner" | "admin" | "editor" => StatusCode::OK,
            "device" | "viewer" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "rename folder", expected);
        let resp = app
            .call(
                "PATCH",
                &folder,
                cred,
                Some(json!({ "retention_max_per_day": 2 })),
            )
            .await;
        let expected = match who {
            "owner" | "admin" => StatusCode::OK,
            "device" | "editor" | "viewer" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "folder retention", expected);
    }

    // Trash and restore: admin and above, web session only. Each actor gets its own
    // folder.
    for (who, cred) in t.actors() {
        let resp = create_folder(
            &app,
            &t.owner,
            &t.scope,
            json!({ "name": format!("bin {who}") }),
        )
        .await;
        let uri = format!("/api/v1/folders/{}", id_of(&resp));
        let resp = app.call("DELETE", &uri, cred, None).await;
        let allowed = matches!(who, "owner" | "admin");
        let expected = match who {
            _ if allowed => StatusCode::NO_CONTENT,
            "device" | "editor" | "viewer" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "trash folder", expected);
        if !allowed {
            app.call("DELETE", &uri, &t.owner, None)
                .await
                .assert(StatusCode::NO_CONTENT);
        }
        // A trashed folder is invisible to editors and viewers.
        let resp = app.post(&format!("{uri}/restore"), cred, json!({})).await;
        let expected = match who {
            _ if allowed => StatusCode::OK,
            "device" => StatusCode::FORBIDDEN,
            _ => StatusCode::NOT_FOUND,
        };
        expect(&resp, who, "restore folder", expected);
        expect(
            &app.get(&uri, cred).await,
            who,
            "get trashed folder",
            if allowed {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            },
        );
    }
}

#[tokio::test]
async fn members_are_managed_by_the_owner_only() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    app.user("new@example.com").await;
    let members = format!("/api/v1/scopes/{}/members", t.scope);
    let editor_id = app.user_id(&t.editor).await;
    let viewer_id = app.user_id(&t.viewer).await;
    let owner_id = app.user_id(&t.owner).await;
    let admin_id = app.user_id(&t.admin).await;

    for (who, cred) in t.actors() {
        if who == "owner" {
            continue;
        }
        let denied = if matches!(who, "outsider" | "site_admin") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::FORBIDDEN
        };
        let resp = app
            .post(&members, cred, json!({ "email": "new@example.com" }))
            .await;
        expect(&resp, who, "add member", denied);
        let resp = app
            .call(
                "PATCH",
                &format!("{members}/{editor_id}"),
                cred,
                Some(json!({ "role": "viewer" })),
            )
            .await;
        expect(&resp, who, "change role", denied);
        // Not its own id: that would be leaving.
        let other = if who == "editor" {
            &viewer_id
        } else {
            &editor_id
        };
        let resp = app
            .call("DELETE", &format!("{members}/{other}"), cred, None)
            .await;
        expect(&resp, who, "remove member", denied);
    }
    // Nor can a scope admin touch the owner.
    app.call("DELETE", &format!("{members}/{owner_id}"), &t.admin, None)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    app.call(
        "PATCH",
        &format!("{members}/{owner_id}"),
        &t.admin,
        Some(json!({ "role": "viewer" })),
    )
    .await
    .assert_error(StatusCode::FORBIDDEN, "forbidden");

    // The owner: change a role, remove a member; the owner itself is no member row.
    let resp = app
        .call(
            "PATCH",
            &format!("{members}/{editor_id}"),
            &t.owner,
            Some(json!({ "role": "viewer" })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["role"], "viewer");
    assert_eq!(resp.body["user"]["email"], "editor@example.com");
    app.call(
        "PATCH",
        &format!("{members}/{owner_id}"),
        &t.owner,
        Some(json!({ "role": "admin" })),
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call(
        "PATCH",
        &format!("{members}/{editor_id}"),
        &t.owner,
        Some(json!({ "role": "owner" })),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    app.call("DELETE", &format!("{members}/{admin_id}"), &t.owner, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/scopes/{}", t.scope), &t.admin)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");
    app.call("DELETE", &format!("{members}/{admin_id}"), &t.owner, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "not_found");

    // The list: owner first, then members by date added.
    let resp = app.get(&members, &t.viewer).await;
    resp.assert(StatusCode::OK);
    let list: Vec<(String, String)> = resp
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["user"]["email"].as_str().unwrap().to_owned(),
                m["role"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let pairs: Vec<(&str, &str)> = list.iter().map(|(e, r)| (e.as_str(), r.as_str())).collect();
    assert_eq!(
        pairs,
        [
            ("user@example.com", "owner"),
            ("editor@example.com", "viewer"),
            ("viewer@example.com", "viewer"),
        ]
    );
    assert_eq!(resp.body[0]["added_by"], json!(null));
    assert_eq!(resp.body[1]["added_by"], json!(owner_id));
}

#[tokio::test]
async fn members_can_leave_but_the_owner_cannot() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let members = format!("/api/v1/scopes/{}/members", t.scope);
    for cred in [&t.viewer, &t.editor, &t.admin] {
        let id = app.user_id(cred).await;
        app.call("DELETE", &format!("{members}/{id}"), cred, None)
            .await
            .assert(StatusCode::NO_CONTENT);
        app.get(&format!("/api/v1/scopes/{}", t.scope), cred)
            .await
            .assert_error(StatusCode::NOT_FOUND, "not_found");
    }
    let owner_id = app.user_id(&t.owner).await;
    app.call("DELETE", &format!("{members}/{owner_id}"), &t.owner, None)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
    // Leaving needs a web session.
    app.call("DELETE", &format!("{members}/{owner_id}"), &t.device, None)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn sharing_by_email() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let owner = app.user("owner@example.com").await;
    app.user("friend@example.com").await;
    let resp = create_scope(&app, &owner, "Games").await;
    let members = format!("/api/v1/scopes/{}/members", id_of(&resp));
    let add = |email: &str| app.post(&members, &owner, json!({ "email": email }));

    // Default role: editor; the email is normalized; sharing again changes the role.
    let resp = add("  Friend@Example.com ").await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["role"], "editor");
    assert_eq!(resp.body["user"]["email"], "friend@example.com");
    let resp = app
        .post(
            &members,
            &owner,
            json!({ "email": "friend@example.com", "role": "viewer" }),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["role"], "viewer");
    assert_eq!(
        app.get(&members, &owner)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Unknown, pending, rejected and disabled accounts all look the same.
    app.set_password_policy(cloudit_core::protocol::NewAccounts::Approval)
        .await;
    app.set_mode(cloudit_core::protocol::RegistrationMode::Open)
        .await;
    app.post(
        "/api/v1/auth/register",
        &As::Anon,
        json!({ "email": "pending@example.com", "password": PASSWORD, "display_name": "P" }),
    )
    .await
    .assert(StatusCode::ACCEPTED);
    let rejected = app.user("rejected@example.com").await;
    let rejected_id = app.user_id(&rejected).await;
    app.post(
        &format!("/api/v1/admin/users/{rejected_id}/reject"),
        &admin,
        json!({}),
    )
    .await
    .assert(StatusCode::OK);
    let disabled = app.user("disabled@example.com").await;
    let disabled_id = app.user_id(&disabled).await;
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{disabled_id}"),
        &admin,
        Some(json!({ "disabled": true })),
    )
    .await
    .assert(StatusCode::OK);
    for email in [
        "nobody@example.com",
        "pending@example.com",
        "rejected@example.com",
        "disabled@example.com",
    ] {
        let resp = add(email).await;
        resp.assert_error(StatusCode::NOT_FOUND, "user_not_found");
    }

    // Yourself (the owner), a malformed email, an unknown role.
    let resp = add("owner@example.com").await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "email");
    add("not an email")
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.post(
        &members,
        &owner,
        json!({ "email": "friend@example.com", "role": "owner" }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
}

#[tokio::test]
async fn adding_members_is_rate_limited() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let owner = app.user("owner@example.com").await;
    let resp = create_scope(&app, &owner, "Games").await;
    let members = format!("/api/v1/scopes/{}/members", id_of(&resp));
    for _ in 0..30 {
        app.post(&members, &owner, json!({ "email": "nobody@example.com" }))
            .await
            .assert_error(StatusCode::NOT_FOUND, "user_not_found");
    }
    let resp = app
        .post(&members, &owner, json!({ "email": "nobody@example.com" }))
        .await;
    resp.assert_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
}

#[tokio::test]
async fn memberships_of_disabled_accounts_stay_but_give_no_access() {
    let app = TestApp::new().await;
    let t = team(&app).await;
    let viewer_id = app.user_id(&t.viewer).await;
    let set_disabled = |on: bool| {
        app.call(
            "PATCH",
            &format!("/api/v1/admin/users/{viewer_id}"),
            &t.site_admin,
            Some(json!({ "disabled": on })),
        )
    };
    set_disabled(true).await.assert(StatusCode::OK);
    let members = app
        .get(&format!("/api/v1/scopes/{}/members", t.scope), &t.owner)
        .await;
    assert_eq!(members.body.as_array().unwrap().len(), 4, "still a member");
    set_disabled(false).await.assert(StatusCode::OK);
    // Disabling deleted the web sessions: sign in again.
    let resp = app.login("viewer@example.com", PASSWORD).await;
    let viewer = As::Cookie(resp.cookie().unwrap());
    let resp = app
        .get(&format!("/api/v1/scopes/{}", t.scope), &viewer)
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["role"], "viewer");
}

#[tokio::test]
async fn names_are_unique_per_owner_ignoring_case() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let alice = app.user("alice@example.com").await;
    let bob = app.user("bob@example.com").await;
    let resp = create_scope(&app, &alice, "  Games ").await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["name"], "Games");
    let games = id_of(&resp);
    create_scope(&app, &alice, "GAMES")
        .await
        .assert_error(StatusCode::CONFLICT, "name_taken");
    create_scope(&app, &bob, "games")
        .await
        .assert(StatusCode::CREATED);
    let resp = create_scope(&app, &alice, "Other").await;
    let other = id_of(&resp);
    app.call(
        "PATCH",
        &format!("/api/v1/scopes/{other}"),
        &alice,
        Some(json!({ "name": "games" })),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "name_taken");
    // Changing only the case of its own name is fine.
    let resp = app
        .call(
            "PATCH",
            &format!("/api/v1/scopes/{games}"),
            &alice,
            Some(json!({ "name": "GAMES" })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["name"], "GAMES");
    for bad in ["", "   ", "a\nb"] {
        let resp = create_scope(&app, &alice, bad).await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(resp.body["field"], "name");
    }
    create_scope(&app, &alice, &"x".repeat(101))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let list = app.get("/api/v1/scopes", &alice).await;
    let names: Vec<&str> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["GAMES", "Other"]);
}

#[tokio::test]
async fn concurrent_creations_have_one_winner() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let alice = app.user("alice@example.com").await;
    let names = [
        "Race", "race", "RACE", "rAce", "Race ", " race", "RaCe", "racE",
    ];
    let results =
        futures::future::join_all(names.iter().map(|n| create_scope(&app, &alice, n))).await;
    let created = results
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    assert_eq!(created, 1, "{results:?}");
    for resp in results.iter().filter(|r| r.status != StatusCode::CREATED) {
        resp.assert_error(StatusCode::CONFLICT, "name_taken");
    }
    // Same for folders in one scope.
    let scope = id_of(
        results
            .iter()
            .find(|r| r.status == StatusCode::CREATED)
            .unwrap(),
    );
    let results = futures::future::join_all(
        names
            .iter()
            .map(|n| create_folder(&app, &alice, &scope, json!({ "name": n }))),
    )
    .await;
    let created = results
        .iter()
        .filter(|r| r.status == StatusCode::CREATED)
        .count();
    assert_eq!(created, 1, "{results:?}");
    for resp in results.iter().filter(|r| r.status != StatusCode::CREATED) {
        resp.assert_error(StatusCode::CONFLICT, "name_taken");
    }
}
