//! Admin endpoints: access, users, last-admin guard, invites, settings.

use axum::http::StatusCode;
use serde_json::json;

use super::support::{As, TestApp};

#[tokio::test]
async fn admin_routes_need_an_admin_session() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;
    for (method, uri) in [
        ("GET", "/api/v1/admin/users"),
        ("GET", "/api/v1/admin/invites"),
        ("POST", "/api/v1/admin/invites"),
        ("GET", "/api/v1/admin/settings"),
        ("PATCH", "/api/v1/admin/settings"),
        ("GET", "/api/v1/admin/summary"),
        (
            "POST",
            "/api/v1/admin/users/00000000-0000-0000-0000-000000000000/approve",
        ),
        (
            "POST",
            "/api/v1/admin/users/00000000-0000-0000-0000-000000000000/reject",
        ),
    ] {
        app.call(method, uri, &user, Some(json!({})))
            .await
            .assert_error(StatusCode::FORBIDDEN, "forbidden");
        app.call(method, uri, &As::Anon, Some(json!({})))
            .await
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
}

#[tokio::test]
async fn list_and_paginate_users() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    for i in 0..3 {
        app.user(&format!("user{i}@example.com")).await;
    }
    let resp = app.get("/api/v1/admin/users", &admin).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["total"], 4);
    let users = resp.body["users"].as_array().unwrap();
    assert_eq!(users[0]["email"], "admin@example.com");
    assert_eq!(users[0]["is_admin"], true);
    assert_eq!(users[0]["admin_from_groups"], false);
    assert!(!resp.body.to_string().contains("argon2"));

    let resp = app
        .get("/api/v1/admin/users?limit=2&offset=1", &admin)
        .await;
    let emails: Vec<&str> = resp.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["email"].as_str().unwrap())
        .collect();
    assert_eq!(emails, ["user0@example.com", "user1@example.com"]);
    app.get("/api/v1/admin/users?limit=0", &admin)
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.get("/api/v1/admin/users?limit=abc", &admin)
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
}

#[tokio::test]
async fn promote_demote_and_last_admin_guard() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let admin_id = app.user_id(&admin).await;
    let user = app.user("user@example.com").await;
    let user_id = app.user_id(&user).await;
    let patch = |id: &str, body: serde_json::Value| {
        app.call(
            "PATCH",
            &format!("/api/v1/admin/users/{id}"),
            &admin,
            Some(body),
        )
    };

    // The only admin can't demote or disable themselves.
    patch(&admin_id, json!({ "admin": false }))
        .await
        .assert_error(StatusCode::CONFLICT, "last_admin");
    patch(&admin_id, json!({ "disabled": true }))
        .await
        .assert_error(StatusCode::CONFLICT, "last_admin");

    // With a second admin, they can.
    let resp = patch(&user_id, json!({ "admin": true })).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["is_admin"], true);
    app.get("/api/v1/admin/users", &user)
        .await
        .assert(StatusCode::OK);
    let resp = patch(&admin_id, json!({ "admin": false })).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["is_admin"], false);
    // No longer admin: refused.
    app.get("/api/v1/admin/users", &admin)
        .await
        .assert_error(StatusCode::FORBIDDEN, "forbidden");

    // The remaining admin is the last one now.
    app.call(
        "PATCH",
        &format!("/api/v1/admin/users/{user_id}"),
        &user,
        Some(json!({ "disabled": true })),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "last_admin");
    app.call(
        "PATCH",
        "/api/v1/admin/users/00000000-0000-0000-0000-000000000000",
        &user,
        Some(json!({ "admin": true })),
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn concurrent_demotions_keep_one_admin() {
    let app = TestApp::new().await;
    let a = app.setup_admin().await;
    let b = app.user("b@example.com").await;
    let (a_id, b_id) = (app.user_id(&a).await, app.user_id(&b).await);
    let promote = app
        .call(
            "PATCH",
            &format!("/api/v1/admin/users/{b_id}"),
            &a,
            Some(json!({ "admin": true })),
        )
        .await;
    promote.assert(StatusCode::OK);

    for round in 0..3 {
        // Each admin demotes the other at the same time: one must lose.
        let (ra, rb) = tokio::join!(
            app.call(
                "PATCH",
                &format!("/api/v1/admin/users/{b_id}"),
                &a,
                Some(json!({ "admin": false }))
            ),
            app.call(
                "PATCH",
                &format!("/api/v1/admin/users/{a_id}"),
                &b,
                Some(json!({ "admin": false }))
            ),
        );
        let ok = [&ra, &rb]
            .iter()
            .filter(|r| r.status == StatusCode::OK)
            .count();
        assert_eq!(ok, 1, "round {round}: {ra:?} {rb:?}");
        let (users, _) = crate::accounts::users::list(&app.state.db, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(users.iter().filter(|u| u.is_admin()).count(), 1);
        // Restore two admins for the next round.
        let (winner, loser_id) = if ra.status == StatusCode::OK {
            (&a, &b_id)
        } else {
            (&b, &a_id)
        };
        app.call(
            "PATCH",
            &format!("/api/v1/admin/users/{loser_id}"),
            winner,
            Some(json!({ "admin": true })),
        )
        .await
        .assert(StatusCode::OK);
    }
}

#[tokio::test]
async fn invites_list_and_revoke() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let resp = app
        .post(
            "/api/v1/admin/invites",
            &admin,
            json!({ "email": "Friend@Example.com", "expires_in": "1d" }),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    let token = resp.body["token"].as_str().unwrap().to_owned();
    assert_eq!(resp.body["path"], format!("/invite/{token}"));
    let id = resp.body["id"].as_str().unwrap().to_owned();
    app.post("/api/v1/admin/invites", &admin, json!({ "email": "nope" }))
        .await
        .assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");

    let resp = app.get("/api/v1/admin/invites", &admin).await;
    resp.assert(StatusCode::OK);
    let list = resp.body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["email"], "friend@example.com");
    assert_eq!(list[0]["status"], "pending");
    assert!(
        !resp.body.to_string().contains(&token),
        "tokens are never listed"
    );

    app.call(
        "DELETE",
        &format!("/api/v1/admin/invites/{id}"),
        &admin,
        None,
    )
    .await
    .assert(StatusCode::NO_CONTENT);
    let resp = app.get("/api/v1/admin/invites", &admin).await;
    assert_eq!(resp.body[0]["status"], "revoked");
    app.call(
        "DELETE",
        "/api/v1/admin/invites/00000000-0000-0000-0000-000000000000",
        &admin,
        None,
    )
    .await
    .assert_error(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn settings_round_trip() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let resp = app.get("/api/v1/admin/settings", &admin).await;
    assert_eq!(
        resp.body,
        json!({
            "registration_mode": "invite", "password_new_accounts": "approval",
            "autolink_email": false, "public_url": null,
            "public_url_from_env": false, "effective_public_url": null
        })
    );
    let resp = app
        .call(
            "PATCH",
            "/api/v1/admin/settings",
            &admin,
            Some(json!({
                "registration_mode": "closed", "autolink_email": true,
                "password_new_accounts": "auto", "public_url": "https://Sync.Example.com/"
            })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(
        resp.body,
        json!({
            "registration_mode": "closed", "password_new_accounts": "auto",
            "autolink_email": true,
            "public_url": "https://sync.example.com", "public_url_from_env": false,
            "effective_public_url": "https://sync.example.com"
        })
    );
    let resp = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(resp.body["registration_mode"], "closed");
    assert_eq!(resp.body["password_requires_approval"], false);
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &admin,
        Some(json!({ "password_new_accounts": "sometimes" })),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    app.call(
        "PATCH",
        "/api/v1/admin/settings",
        &admin,
        Some(json!({ "registration_mode": "maybe" })),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    // An invalid URL changes nothing (not even the other fields).
    let resp = app
        .call(
            "PATCH",
            "/api/v1/admin/settings",
            &admin,
            Some(json!({ "autolink_email": false, "public_url": "https://x.example.com/app" })),
        )
        .await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "public_url");
    let resp = app.get("/api/v1/admin/settings", &admin).await;
    assert_eq!(resp.body["autolink_email"], true);
    // Empty string clears the URL.
    let resp = app
        .call(
            "PATCH",
            "/api/v1/admin/settings",
            &admin,
            Some(json!({ "public_url": "" })),
        )
        .await;
    assert_eq!(resp.body["public_url"], json!(null));
    assert_eq!(resp.body["effective_public_url"], json!(null));
}

#[tokio::test]
async fn env_public_url_wins_over_the_stored_one() {
    let app = TestApp::with(&[("PUBLIC_URL", "https://env.example.com")]).await;
    let admin = app.setup_admin().await;
    let resp = app
        .call(
            "PATCH",
            "/api/v1/admin/settings",
            &admin,
            Some(json!({ "public_url": "https://db.example.com" })),
        )
        .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["public_url"], "https://db.example.com");
    assert_eq!(resp.body["public_url_from_env"], true);
    assert_eq!(resp.body["effective_public_url"], "https://env.example.com");
}
