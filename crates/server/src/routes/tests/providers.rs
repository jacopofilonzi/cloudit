//! Identity-provider admin API: presets, CRUD, discovery, secrets, soft delete.

use axum::http::StatusCode;
use sea_orm::EntityTrait as _;
use serde_json::{Value, json};

use super::fake_idp::FakeIdp;
use super::support::{As, TestApp};
use crate::db::entities::auth_providers;

const SECRET: &str = "super-secret-client-value";

fn oidc_body(id: &str, issuer: &str) -> Value {
    json!({
        "id": id, "kind": "keycloak", "issuer_url": issuer,
        "client_id": "cloudit", "client_secret": SECRET
    })
}

#[tokio::test]
async fn provider_routes_need_an_admin() {
    let app = TestApp::new().await;
    app.setup_admin().await;
    let user = app.user("user@example.com").await;
    for (method, uri) in [
        ("GET", "/api/v1/admin/providers"),
        ("POST", "/api/v1/admin/providers"),
        ("GET", "/api/v1/admin/providers/presets"),
        ("PATCH", "/api/v1/admin/providers/x"),
        ("DELETE", "/api/v1/admin/providers/x"),
        ("POST", "/api/v1/admin/providers/x/test"),
    ] {
        app.call(method, uri, &user, Some(json!({})))
            .await
            .assert_error(StatusCode::FORBIDDEN, "forbidden");
    }
}

#[tokio::test]
async fn presets_list_every_kind() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    let resp = app.get("/api/v1/admin/providers/presets", &admin).await;
    resp.assert(StatusCode::OK);
    let kinds: Vec<&str> = resp
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "google",
            "microsoft",
            "gitlab",
            "authentik",
            "keycloak",
            "authelia",
            "zitadel",
            "pocketid",
            "oidc",
            "github",
            "discord"
        ]
    );
    let google = &resp.body[0];
    assert_eq!(google["issuer_url"], "https://accounts.google.com");
    assert_eq!(google["issuer_editable"], false);
    assert_eq!(google["protocol"], "oidc");
    let microsoft = &resp.body[1];
    assert_eq!(microsoft["tenant_required"], true);
    let github = &resp.body[9];
    assert_eq!(github["protocol"], "oauth2");
    assert_eq!(github["default_scopes"], "read:user user:email");
    assert_eq!(github["supports_groups"], false);
    assert_eq!(github["new_accounts"], "approval");
    assert_eq!(resp.body[8]["icon"], "generic");
}

#[tokio::test]
async fn create_list_update_and_soft_delete() {
    let idp = FakeIdp::start().await;
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;

    let resp = app
        .post(
            "/api/v1/admin/providers",
            &admin,
            oidc_body("company-sso", &idp.issuer()),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["id"], "company-sso");
    assert_eq!(resp.body["kind"], "keycloak");
    assert_eq!(resp.body["display_name"], "Keycloak");
    assert_eq!(resp.body["icon"], "keycloak");
    assert_eq!(resp.body["scopes"], "openid email profile");
    assert_eq!(resp.body["has_secret"], true);
    assert_eq!(resp.body["enabled"], true);
    assert_eq!(resp.body["position"], 0);
    assert_eq!(resp.body["new_accounts"], "approval", "default");
    assert_eq!(resp.body["callback_url"], Value::Null, "no public URL yet");
    assert!(!resp.body.to_string().contains(SECRET));

    // The secret is stored encrypted, bound to the provider id.
    let row = auth_providers::Entity::find_by_id("company-sso".to_owned())
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&row.client_secret_enc).contains(SECRET));
    let provider = crate::idp::store::find(&app.state.db, "company-sso")
        .await
        .unwrap();
    assert_eq!(
        provider.client_secret(&app.state.secrets).unwrap().expose(),
        SECRET
    );

    // A GitHub provider (no discovery), placed after it.
    let resp = app
        .post(
            "/api/v1/admin/providers",
            &admin,
            json!({
                "id": "gh", "kind": "github", "client_id": "Iv1", "client_secret": "s",
                "new_accounts": "auto"
            }),
        )
        .await;
    resp.assert(StatusCode::CREATED);
    assert_eq!(resp.body["position"], 1);
    assert_eq!(resp.body["new_accounts"], "auto");

    // Ids are unique, deleted ones included.
    app.post(
        "/api/v1/admin/providers",
        &admin,
        oidc_body("gh", &idp.issuer()),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "provider_id_taken");

    // The callback URL appears once a public URL is known.
    crate::accounts::settings::set_public_url(
        &app.state.db,
        Some("https://sync.example.com"),
        None,
    )
    .await
    .unwrap();
    let resp = app.get("/api/v1/admin/providers", &admin).await;
    resp.assert(StatusCode::OK);
    let list = resp.body.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[0]["callback_url"],
        "https://sync.example.com/api/v1/auth/oauth/company-sso/callback"
    );
    assert!(!resp.body.to_string().contains(SECRET));

    // Login page: enabled ones, in position order.
    let config = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(
        config.body["providers"],
        json!([
            { "id": "company-sso", "kind": "keycloak", "display_name": "Keycloak", "icon": "keycloak" },
            { "id": "gh", "kind": "github", "display_name": "GitHub", "icon": "github" }
        ])
    );

    // Update: reorder, rename, disable, rotate the secret; id and kind can't change.
    let patch = |id: &str, body: Value| {
        app.call(
            "PATCH",
            &format!("/api/v1/admin/providers/{id}"),
            &admin,
            Some(body),
        )
    };
    let resp = patch(
        "company-sso",
        json!({ "display_name": "Company SSO", "position": 5, "client_secret": "rotated",
                "admin_groups": ["admins"], "groups_claim": "roles" }),
    )
    .await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["display_name"], "Company SSO");
    assert_eq!(resp.body["admin_groups"], json!(["admins"]));
    assert_eq!(resp.body["groups_claim"], "roles");
    assert_eq!(resp.body["new_accounts"], "approval", "unchanged");
    let resp = patch("company-sso", json!({ "new_accounts": "auto" })).await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body["new_accounts"], "auto");
    let listed = app.get("/api/v1/admin/providers", &admin).await;
    assert_eq!(listed.body[0]["new_accounts"], "auto");
    patch("company-sso", json!({ "new_accounts": "never" }))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    let provider = crate::idp::store::find(&app.state.db, "company-sso")
        .await
        .unwrap();
    assert_eq!(
        provider.client_secret(&app.state.secrets).unwrap().expose(),
        "rotated"
    );
    patch("gh", json!({ "enabled": false }))
        .await
        .assert(StatusCode::OK);
    let config = app.get("/api/v1/auth/config", &As::Anon).await;
    assert_eq!(config.body["providers"].as_array().unwrap().len(), 1);
    patch("gh", json!({ "kind": "discord" }))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    patch("gh", json!({ "id": "other" }))
        .await
        .assert_error(StatusCode::BAD_REQUEST, "bad_request");
    let resp = patch("gh", json!({ "admin_groups": ["x"] })).await;
    resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(resp.body["field"], "admin_groups");
    patch("nope", json!({ "enabled": true }))
        .await
        .assert_error(StatusCode::NOT_FOUND, "provider_not_found");

    // Soft delete: hidden, disabled, the row and its id stay.
    app.call("DELETE", "/api/v1/admin/providers/gh", &admin, None)
        .await
        .assert(StatusCode::NO_CONTENT);
    app.call("DELETE", "/api/v1/admin/providers/gh", &admin, None)
        .await
        .assert_error(StatusCode::NOT_FOUND, "provider_not_found");
    patch("gh", json!({ "enabled": true }))
        .await
        .assert_error(StatusCode::NOT_FOUND, "provider_not_found");
    let resp = app.get("/api/v1/admin/providers", &admin).await;
    assert_eq!(resp.body.as_array().unwrap().len(), 1);
    let row = auth_providers::Entity::find_by_id("gh".to_owned())
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    assert!(row.deleted_at.is_some());
    assert!(!row.enabled);
    app.post(
        "/api/v1/admin/providers",
        &admin,
        json!({ "id": "gh", "kind": "github", "client_id": "Iv1", "client_secret": "s" }),
    )
    .await
    .assert_error(StatusCode::CONFLICT, "provider_id_taken");
}

#[tokio::test]
async fn discovery_must_succeed() {
    let idp = FakeIdp::start().await;
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;

    // Unreachable issuer, broken discovery, issuer mismatch: refused, nothing stored.
    for issuer in [
        "http://127.0.0.1:1/realms/x".to_owned(),
        format!("{}/other", idp.base),
        format!("{}/", idp.issuer()),
    ] {
        app.post("/api/v1/admin/providers", &admin, oidc_body("kc", &issuer))
            .await
            .assert_error(StatusCode::BAD_GATEWAY, "provider_discovery_failed");
    }
    idp.set_broken(true);
    app.post(
        "/api/v1/admin/providers",
        &admin,
        oidc_body("kc", &idp.issuer()),
    )
    .await
    .assert_error(StatusCode::BAD_GATEWAY, "provider_discovery_failed");
    assert!(
        auth_providers::Entity::find()
            .all(&app.state.db)
            .await
            .unwrap()
            .is_empty()
    );

    idp.set_broken(false);
    app.post(
        "/api/v1/admin/providers",
        &admin,
        oidc_body("kc", &idp.issuer()),
    )
    .await
    .assert(StatusCode::CREATED);
    let test = || app.post("/api/v1/admin/providers/kc/test", &admin, json!({}));
    let resp = test().await;
    resp.assert(StatusCode::OK);
    assert_eq!(resp.body, json!({ "ok": true, "code": null }));
    idp.set_broken(true);
    let resp = test().await;
    assert_eq!(
        resp.body,
        json!({ "ok": false, "code": "provider_discovery_failed" })
    );
    // Changing the issuer re-runs discovery.
    let resp = app
        .call(
            "PATCH",
            "/api/v1/admin/providers/kc",
            &admin,
            Some(json!({ "issuer_url": "http://127.0.0.1:1/x" })),
        )
        .await;
    resp.assert_error(StatusCode::BAD_GATEWAY, "provider_discovery_failed");
    // Other changes don't contact the provider.
    app.call(
        "PATCH",
        "/api/v1/admin/providers/kc",
        &admin,
        Some(json!({ "display_name": "SSO" })),
    )
    .await
    .assert(StatusCode::OK);
}

#[tokio::test]
async fn create_validation() {
    let app = TestApp::new().await;
    let admin = app.setup_admin().await;
    for (body, field) in [
        (
            json!({ "id": "A", "kind": "github", "client_id": "c", "client_secret": "s" }),
            "id",
        ),
        (
            json!({ "id": "gh", "kind": "github", "client_id": "c", "client_secret": "" }),
            "client_secret",
        ),
        (
            json!({ "id": "gh", "kind": "github", "client_id": " ", "client_secret": "s" }),
            "client_id",
        ),
        (
            json!({ "id": "ms", "kind": "microsoft", "tenant": "common", "client_id": "c", "client_secret": "s" }),
            "tenant",
        ),
        (
            json!({ "id": "kc", "kind": "keycloak", "client_id": "c", "client_secret": "s" }),
            "issuer_url",
        ),
        (
            json!({ "id": "gh", "kind": "github", "icon": "apple", "client_id": "c", "client_secret": "s" }),
            "icon",
        ),
    ] {
        let resp = app.post("/api/v1/admin/providers", &admin, body).await;
        resp.assert_error(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(resp.body["field"], field);
    }
    app.post(
        "/api/v1/admin/providers",
        &admin,
        json!({ "id": "ap", "kind": "apple", "client_id": "c", "client_secret": "s" }),
    )
    .await
    .assert_error(StatusCode::BAD_REQUEST, "bad_request");
}
