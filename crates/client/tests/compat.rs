//! Compatibility with older and newer servers (`/health`: API level, minimum app
//! version).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use cloudit_client::compat::{APP_VERSION, REQUIRED_API_LEVEL};
use cloudit_client::events::{RemoteEvent, follow};
use cloudit_client::{CancellationToken, Compat, ErrorKind, OpContext};
use common::*;
use serde_json::json;

fn ctx() -> OpContext {
    OpContext::new()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_real_server_is_compatible() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    assert_eq!(a.engine.compatibility(), None);
    a.engine.scopes(&ctx()).await.unwrap();
    // The operation checked and cached the answer.
    assert_eq!(a.engine.compatibility(), Some(Compat::Ok));
    assert_eq!(
        a.engine.check_compatibility(&ctx()).await.unwrap(),
        Compat::Ok
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_old_server_stops_the_operations() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    server.set_health(Some(json!({
        "status": "ok", "version": "0.0.9", "api_level": REQUIRED_API_LEVEL - 1,
        "min_client_version": "0.0.1"
    })));
    assert_eq!(
        a.engine.check_compatibility(&ctx()).await.unwrap(),
        Compat::ServerOutdated {
            server_version: "0.0.9".to_owned(),
            api_level: REQUIRED_API_LEVEL - 1,
            required_api_level: REQUIRED_API_LEVEL,
        }
    );
    let err = a.engine.scopes(&ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ServerOutdated);
    assert!(matches!(err.compat(), Some(Compat::ServerOutdated { .. })));

    // A server from before the fields existed counts as level 1 with no minimum.
    server.set_health(Some(json!({ "status": "ok", "version": "0.1.0" })));
    assert_eq!(
        a.engine.check_compatibility(&ctx()).await.unwrap(),
        Compat::Ok
    );
    a.engine.scopes(&ctx()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_newer_server_refuses_an_old_app_at_sign_in() {
    let server = Server::start().await;
    server.set_health(Some(json!({
        "status": "ok", "version": "99.0.0", "api_level": 99, "min_client_version": "99.0.0"
    })));
    let data = tempfile::tempdir().unwrap();
    let engine = open_engine(data.path(), None).await;
    let err = engine
        .begin_login(&server.url, Some("Desk"), &ctx())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AppOutdated);
    assert_eq!(
        err.compat(),
        Some(Compat::AppOutdated {
            app_version: APP_VERSION.to_owned(),
            min_client_version: "99.0.0".to_owned(),
        })
    );
    assert!(!engine.signed_in());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_follower_reports_an_incompatible_server_and_holds_off() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    server.set_health(Some(json!({
        "status": "ok", "version": "99.0.0", "api_level": 99, "min_client_version": "99.0.0"
    })));
    let cancel = CancellationToken::new();
    let mut rx = follow(&a.engine, cancel.clone());
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            &first,
            RemoteEvent::Compatibility {
                compat: Compat::AppOutdated { .. }
            }
        ),
        "{first:?}"
    );
    // No stream, no reconnect loop: nothing else comes for a while.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err()
    );
    cancel.cancel();
}
