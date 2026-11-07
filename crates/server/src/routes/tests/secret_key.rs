//! Server key at startup: auto-generated file (SQLite), required with PostgreSQL, and
//! the check value that refuses a changed key.

use crate::config::Config;
use crate::crypto::key::KEY_FILE;
use crate::db::testing::TestDb;
use crate::state::AppState;

const KEY_A: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const KEY_B: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";

fn config(db: &TestDb, key: Option<&str>) -> Config {
    let mut vars = vec![(
        "DATA_DIR".to_owned(),
        db.dir.path().to_string_lossy().into_owned(),
    )];
    if let Some(url) = db.url() {
        vars.push(("DATABASE_URL".to_owned(), url.to_owned()));
    }
    if let Some(key) = key {
        vars.push(("SECRET_KEY".to_owned(), key.to_owned()));
    }
    Config::from_lookup(|k| {
        vars.iter()
            .find(|(name, _)| name == k)
            .map(|(_, v)| v.clone())
    })
    .unwrap()
}

async fn init(db: &TestDb, key: Option<&str>) -> anyhow::Result<AppState> {
    AppState::init(config(db, key)).await
}

#[tokio::test]
async fn changed_key_refuses_to_start() {
    let db = TestDb::empty().await;
    let first = init(&db, Some(KEY_A)).await.unwrap();
    first.db.close().await.unwrap();
    // Same key: fine, any number of times.
    init(&db, Some(KEY_A))
        .await
        .unwrap()
        .db
        .close()
        .await
        .unwrap();
    let err = init(&db, Some(KEY_B)).await.err().unwrap();
    assert!(
        format!("{err:#}").contains("SECRET_KEY changed or wrong"),
        "{err:#}"
    );
}

#[tokio::test]
async fn sqlite_generates_and_reuses_a_key_file() {
    if crate::db::testing::test_database_url().is_some() {
        return; // SQLite only.
    }
    let db = TestDb::empty().await;
    let path = db.dir.path().join(KEY_FILE);
    let state = init(&db, None).await.unwrap();
    state.db.close().await.unwrap();
    let key = std::fs::read_to_string(&path).unwrap();
    assert_eq!(key.trim().len(), 44);
    // Restart: same file, check passes.
    init(&db, None).await.unwrap().db.close().await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), key);
    // The file was lost and a new one generated: refused.
    std::fs::remove_file(&path).unwrap();
    let err = init(&db, None).await.err().unwrap();
    assert!(
        format!("{err:#}").contains("SECRET_KEY changed or wrong"),
        "{err:#}"
    );
}

#[tokio::test]
async fn postgres_requires_secret_key() {
    if crate::db::testing::test_database_url().is_none() {
        return; // PostgreSQL only (`make test-ha`).
    }
    let db = TestDb::empty().await;
    let err = init(&db, None).await.err().unwrap();
    assert!(
        format!("{err:#}").contains("SECRET_KEY is required"),
        "{err:#}"
    );
    assert!(!db.dir.path().join(KEY_FILE).exists());
}
