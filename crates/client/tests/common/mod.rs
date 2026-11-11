//! Test harness: the real cloudit server in-process on `127.0.0.1:<random>` (SQLite and
//! blobs in a temp dir, first-run setup done), and engines signed in to it through the
//! real browser flow (a fake browser approves on the `/device` page's behalf).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cloudit_client::api::RetryPolicy;
use cloudit_client::{
    Engine, EngineConfig, LinkedFolder, MemoryBackend, OpContext, SecretBackend, TokenVault,
};
use cloudit_server::config::Config;
use cloudit_server::routes::router;
use cloudit_server::state::AppState;
use serde_json::{Value, json};
use tempfile::TempDir;

pub const SETUP_TOKEN: &str = "client-tests-setup-token-0123456789";
pub const ADMIN_EMAIL: &str = "admin@example.com";
pub const PASSWORD: &str = "correct horse battery";

/// A running server with its admin session.
pub struct Server {
    pub url: String,
    pub state: AppState,
    pub http: reqwest::Client,
    cookie: String,
    health: HealthOverride,
    requests: RequestLog,
    task: tokio::task::JoinHandle<()>,
    _dir: TempDir,
}

/// A replacement body for the health endpoints (see [`Server::set_health`]).
type HealthOverride = Arc<Mutex<Option<Value>>>;
/// `"<METHOD> <path>"` of every request served (see [`Server::requests`]).
type RequestLog = Arc<Mutex<Vec<String>>>;

async fn override_health(
    State((health, requests)): State<(HealthOverride, RequestLog)>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    requests
        .lock()
        .unwrap()
        .push(format!("{} {path}", request.method()));
    if path == "/health" || path == "/api/v1/health" {
        let body = health.lock().unwrap().clone();
        if let Some(body) = body {
            return axum::Json(body).into_response();
        }
    }
    next.run(request).await
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start() -> Self {
        Self::start_with(&[]).await
    }

    /// With extra environment variables.
    pub async fn start_with(extra: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mut vars: Vec<(String, String)> = vec![
            ("DATA_DIR".into(), dir.path().to_string_lossy().into_owned()),
            (
                "WEB_DIR".into(),
                dir.path().join("no-web").to_string_lossy().into_owned(),
            ),
            ("SETUP_TOKEN".into(), SETUP_TOKEN.into()),
            ("PUBLIC_URL".into(), url.clone()),
        ];
        for (k, v) in extra {
            vars.retain(|(key, _)| key != k);
            if !v.is_empty() {
                vars.push(((*k).to_owned(), (*v).to_owned()));
            }
        }
        let config =
            Config::from_lookup(|key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
                .unwrap();
        let state = AppState::init(config).await.unwrap();
        let health = HealthOverride::default();
        let requests = RequestLog::default();
        let app = router(state.clone()).layer(axum::middleware::from_fn_with_state(
            (health.clone(), requests.clone()),
            override_health,
        ));
        let task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let setup = http
            .post(format!("{url}/api/v1/setup"))
            .header("origin", &url)
            .header("content-type", "application/json")
            .body(
                json!({
                    "token": SETUP_TOKEN, "email": ADMIN_EMAIL, "password": PASSWORD,
                    "display_name": "Admin"
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(setup.status(), 201, "setup failed");
        let cookie = setup
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("cloudit_session="))
            .and_then(|v| v.split(';').next())
            .unwrap()
            .to_owned();
        Self {
            url,
            state,
            http,
            cookie,
            health,
            requests,
            task,
            _dir: dir,
        }
    }

    /// Makes `/health` and `/api/v1/health` answer `body` (`None`: the real answer),
    /// e.g. to play an older or newer server.
    pub fn set_health(&self, body: Option<Value>) {
        *self.health.lock().unwrap() = body;
    }

    /// Takes the log of the requests served so far (`"<METHOD> <path>"`).
    pub fn take_requests(&self) -> Vec<String> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }

    /// A JSON call with the admin's web session; the status and the body.
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self
            .http
            .request(
                method.parse().unwrap(),
                format!("{}/api/v1{path}", self.url),
            )
            .header("origin", &self.url)
            .header("cookie", &self.cookie);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Plays the browser on the `/device` page: allows the device, then follows the
    /// redirect to the loopback listener.
    pub async fn approve(&self, browser_url: &str) {
        let url = url::Url::parse(browser_url).unwrap();
        assert_eq!(url.path(), "/device");
        let q = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .unwrap()
                .1
                .into_owned()
        };
        let (status, body) = self
            .call(
                "POST",
                "/devices/authorize",
                Some(json!({
                    "code_challenge": q("code_challenge"),
                    "code_challenge_method": q("code_challenge_method"),
                    "redirect_uri": q("redirect_uri"),
                    "state": q("state"),
                    "device_name": q("device_name"),
                    "os": q("os"),
                    "token_ttl": "30d",
                })),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let redirect = body["redirect_url"].as_str().unwrap();
        let back = self.http.get(redirect).send().await.unwrap();
        assert_eq!(back.status(), 200);
    }

    /// The paths of a folder's head manifest.
    pub async fn head_paths(&self, folder_id: &str) -> Vec<String> {
        let (_, folder) = self
            .call("GET", &format!("/folders/{folder_id}"), None)
            .await;
        let Some(head) = folder["head_snapshot_id"].as_str() else {
            return Vec::new();
        };
        let (status, manifest) = self
            .call(
                "GET",
                &format!("/snapshots/{head}/manifest?format=json"),
                None,
            )
            .await;
        assert_eq!(status, 200);
        manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_owned())
            .collect()
    }

    /// A folder's snapshots, newest first.
    pub async fn snapshots(&self, folder_id: &str) -> Vec<Value> {
        let (status, list) = self
            .call(
                "GET",
                &format!("/folders/{folder_id}/snapshots?limit=200"),
                None,
            )
            .await;
        assert_eq!(status, 200);
        list["snapshots"].as_array().unwrap().clone()
    }

    /// The server folder.
    pub async fn folder(&self, folder_id: &str) -> Value {
        let (status, folder) = self
            .call("GET", &format!("/folders/{folder_id}"), None)
            .await;
        assert_eq!(status, 200);
        folder
    }
}

/// A signed-in engine with its own data directory.
pub struct Client {
    pub engine: Engine,
    pub data: TempDir,
}

/// Fast retries: tests must not wait for backoffs.
pub fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_retries: 1,
        base_delay: Duration::from_millis(10),
    }
}

/// An engine on `data` with an in-memory keychain (or `keychain`).
pub async fn open_engine(data: &Path, keychain: Option<Box<dyn SecretBackend>>) -> Engine {
    let mut config = EngineConfig::new(data);
    config.retry = fast_retry();
    config.token_vault = Some(TokenVault::with_backend(
        Some(keychain.unwrap_or_else(|| Box::new(MemoryBackend::default()))),
        data,
    ));
    Engine::open(config).await.unwrap()
}

/// Signs `engine` in to `server` as the admin, through the browser flow.
pub async fn sign_in(server: &Server, engine: &Engine, device: &str) {
    let ctx = OpContext::new();
    let pending = engine
        .begin_login(&server.url, Some(device), &ctx)
        .await
        .unwrap();
    let browser_url = pending.browser_url().to_owned();
    let (account, ()) = tokio::join!(
        engine.finish_login(pending, &ctx),
        server.approve(&browser_url)
    );
    let account = account.unwrap();
    assert_eq!(account.email, ADMIN_EMAIL);
    assert_eq!(account.device_name, device);
}

/// A new signed-in client.
pub async fn client(server: &Server, device: &str) -> Client {
    let data = tempfile::tempdir().unwrap();
    let engine = open_engine(data.path(), None).await;
    sign_in(server, &engine, device).await;
    Client { engine, data }
}

/// Creates a scope + folder and links it on `a` at `path`.
pub async fn new_folder(a: &Client, path: &Path, ignore: &[&str]) -> LinkedFolder {
    let ctx = OpContext::new();
    let scope = a.engine.create_scope("Games", &ctx).await.unwrap();
    a.engine
        .create_and_link(&scope.id, "Saves", Some("🎮"), path, &strings(ignore), &ctx)
        .await
        .unwrap()
}

/// Links the server folder of `link` on `b` at `path`.
pub async fn link_on(
    b: &Client,
    link: &LinkedFolder,
    path: &Path,
    ignore: &[&str],
) -> LinkedFolder {
    b.engine
        .link_folder(&link.folder_id, path, &strings(ignore), &OpContext::new())
        .await
        .unwrap()
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Writes `content` to `root/rel` (creating parents).
pub fn write(root: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let path = full(root, rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn read(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(full(root, rel)).unwrap()
}

pub fn exists(root: &Path, rel: &str) -> bool {
    full(root, rel).exists()
}

pub fn full(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

/// Every file and directory under `root`, relative with `/`, sorted.
pub fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walkdir_list(root) {
        out.push(entry);
    }
    out.sort();
    out
}

fn walkdir_list(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
                out.push(format!("{rel}/"));
            } else {
                out.push(rel);
            }
        }
    }
    out
}

/// Deterministic pseudo-random bytes (SplitMix64).
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.extend_from_slice(&z.to_le_bytes());
    }
    out.truncate(len);
    out
}
