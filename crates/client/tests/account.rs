//! Sign-in, token storage and folder links.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use cloudit_client::{Engine, ErrorKind, LinkRefusal, OpContext, SecretBackend, TokenLocation};
use common::*;

fn ctx() -> OpContext {
    OpContext::new()
}

/// A keychain that is never available (a Linux session without Secret Service).
struct NoKeychain;

impl SecretBackend for NoKeychain {
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("org.freedesktop.secrets was not provided".to_owned())
    }
    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Err("org.freedesktop.secrets was not provided".to_owned())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("org.freedesktop.secrets was not provided".to_owned())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_falls_back_to_a_private_file_and_survives_a_restart() {
    let server = Server::start().await;
    let data = tempfile::tempdir().unwrap();
    let engine = open_engine(data.path(), Some(Box::new(NoKeychain))).await;
    assert!(!engine.signed_in());
    assert_eq!(
        engine.scopes(&ctx()).await.unwrap_err().kind(),
        ErrorKind::NotSignedIn
    );
    sign_in(&server, &engine, "Steam Deck").await;
    let account = engine.account().unwrap().unwrap();
    assert_eq!(account.token_location, TokenLocation::File);
    assert!(account.token_warning());
    assert!(data.path().join("token.json").exists());
    // The token never lands in the state database.
    let state = std::fs::read(data.path().join("state.db")).unwrap();
    let token_file = std::fs::read_to_string(data.path().join("token.json")).unwrap();
    let token = token_file
        .split("cld_")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(!state.windows(token.len()).any(|w| w == token.as_bytes()));
    drop(engine);

    let reopened = open_engine(data.path(), Some(Box::new(NoKeychain))).await;
    assert!(reopened.signed_in());
    reopened.scopes(&ctx()).await.unwrap();
    reopened.logout().await.unwrap();
    assert!(!data.path().join("token.json").exists());
    assert!(reopened.account().unwrap().is_none());
    assert!(!reopened.signed_in());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_is_unauthorized() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let device = a.engine.account().unwrap().unwrap().device_id;
    let (status, _) = server
        .call("DELETE", &format!("/devices/{device}"), None)
        .await;
    assert_eq!(status, 204);
    let err = a.engine.scopes(&ctx()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unauthorized);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn link_rules() {
    let server = Server::start().await;
    let a = client(&server, "Desk").await;
    let root = tempfile::tempdir().unwrap();
    let games = root.path().join("games");
    let link = new_folder(&a, &games, &[]).await;
    assert!(games.is_dir());

    let refused = |err: cloudit_client::Error| match err {
        cloudit_client::Error::LinkRefused { reason, .. } => reason,
        other => panic!("{other:?}"),
    };
    let scope = a.engine.scopes(&ctx()).await.unwrap().remove(0);
    let other = a
        .engine
        .create_folder(&scope.id, "Other", None, &ctx())
        .await
        .unwrap();
    let try_link = |path: std::path::PathBuf| {
        let engine = a.engine.clone();
        let id = other.id.clone();
        async move { engine.link_folder(&id, &path, &[], &ctx()).await }
    };
    assert_eq!(
        refused(try_link(games.clone()).await.unwrap_err()),
        LinkRefusal::AlreadyLinked
    );
    assert_eq!(
        refused(try_link(games.join("inside")).await.unwrap_err()),
        LinkRefusal::Nested
    );
    assert_eq!(
        refused(try_link(root.path().to_path_buf()).await.unwrap_err()),
        LinkRefusal::Nested
    );
    assert_eq!(
        refused(try_link(a.engine.data_dir().join("x")).await.unwrap_err()),
        LinkRefusal::DataDir
    );
    let file = root.path().join("file.txt");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        refused(try_link(file).await.unwrap_err()),
        LinkRefusal::NotADirectory
    );
    assert_eq!(
        refused(
            a.engine
                .link_folder(&link.folder_id, &root.path().join("again"), &[], &ctx())
                .await
                .unwrap_err()
        ),
        LinkRefusal::FolderAlreadyLinked
    );
    assert_eq!(
        a.engine
            .set_ignore(link.id, &strings(&["!keep"]))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );

    // Unlink keeps the files; relocate follows a moved folder.
    write(&games, "save.dat", b"1");
    a.engine.sync(link.id, &ctx()).await.unwrap();
    let moved = root.path().join("moved");
    std::fs::rename(&games, &moved).unwrap();
    assert_eq!(
        a.engine.status(link.id, &ctx()).await.unwrap().state,
        cloudit_client::SyncState::MissingLocalPath
    );
    let relinked = a.engine.relocate(link.id, &moved).await.unwrap();
    assert_eq!(
        a.engine.status(relinked.id, &ctx()).await.unwrap().state,
        cloudit_client::SyncState::InSync
    );
    a.engine.unlink(link.id).await.unwrap();
    assert!(a.engine.folders().unwrap().is_empty());
    assert_eq!(read(&moved, "save.dat"), b"1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_account_unlinks_the_folders() {
    let first = Server::start().await;
    let second = Server::start().await;
    let data = tempfile::tempdir().unwrap();
    let engine: Engine = open_engine(data.path(), None).await;
    sign_in(&first, &engine, "Desk").await;
    let dir = tempfile::tempdir().unwrap();
    let client = Client {
        engine: engine.clone(),
        data,
    };
    new_folder(&client, dir.path(), &[]).await;
    assert_eq!(engine.folders().unwrap().len(), 1);
    // Signing in again to the same account keeps them.
    sign_in(&first, &engine, "Desk").await;
    assert_eq!(engine.folders().unwrap().len(), 1);
    sign_in(&second, &engine, "Desk").await;
    assert!(engine.folders().unwrap().is_empty());
    assert_eq!(engine.account().unwrap().unwrap().server_url, second.url);
}
