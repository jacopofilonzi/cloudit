//! [`Engine`]: the public façade the desktop app (and a future CLI) drives.
//!
//! - account: [`Engine::begin_login`] / [`Engine::finish_login`] (browser sign-in),
//!   [`Engine::account`], [`Engine::logout`];
//! - server listings: [`Engine::scopes`], [`Engine::server_folders`],
//!   [`Engine::create_scope`], [`Engine::create_folder`];
//! - links (`linking.rs`): [`Engine::link_folder`], [`Engine::create_and_link`],
//!   [`Engine::unlink`], [`Engine::relocate`], [`Engine::set_ignore`],
//!   [`Engine::set_paused`], [`Engine::folders`], [`Engine::refresh_names`];
//! - sync (`ops.rs`): [`Engine::status`], [`Engine::sync`], and the conflict decisions
//!   [`Engine::keep_local`], [`Engine::keep_server`], [`Engine::decide_later`];
//! - [`Engine::activity`]; events: [`crate::events::follow`].
//!
//! `Engine` is cheap to clone (an `Arc`) and `Send + Sync`. Operations on one linked
//! folder are serialized by a per-folder lock (a second call waits); different folders
//! run in parallel.

mod linking;
mod ops;

pub use ops::{SyncOutcome, SyncReport};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use cloudit_core::protocol::{
    CreateFolderRequest, DeviceTokenRequest, FolderInfo, ScopeInfo, Sensitive,
};

use crate::api::{self, Api, RetryPolicy};
use crate::auth::login::{DEFAULT_LOGIN_TIMEOUT, PendingLogin};
use crate::auth::tokens::TokenVault;
use crate::error::{Error, Result};
use crate::platform;
use crate::progress::OpContext;
use crate::secret::Secret;
use crate::state::Db;
use crate::state::account::Account;
use crate::state::activity::{ActivityEntry, NewActivity};

/// `meta` key: the account the folder links belong to.
const LAST_ACCOUNT: &str = "last_account";

/// How to open an [`Engine`].
pub struct EngineConfig {
    /// The engine's own directory: state database, chunk cache, token fallback file.
    /// Never inside a synced folder.
    pub data_dir: PathBuf,
    /// Token storage; default: the OS keychain with a file fallback in `data_dir`.
    pub token_vault: Option<TokenVault>,
    /// Retries of failed requests.
    pub retry: RetryPolicy,
    /// Time the user has to finish the browser sign-in.
    pub login_timeout: std::time::Duration,
}

impl EngineConfig {
    /// Defaults for `data_dir` (see [`platform::default_data_dir`] for the usual one).
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            token_vault: None,
            retry: RetryPolicy::default(),
            login_timeout: DEFAULT_LOGIN_TIMEOUT,
        }
    }
}

struct Session {
    account: Account,
    api: Api,
}

pub(crate) struct Inner {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    db: Db,
    vault: TokenVault,
    http: reqwest::Client,
    retry: RetryPolicy,
    login_timeout: std::time::Duration,
    session: RwLock<Option<Session>>,
    locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

/// The sync engine (see the module docs).
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("data_dir", &self.inner.data_dir)
            .finish_non_exhaustive()
    }
}

/// The token's key in the vault.
fn token_key(account: &Account) -> String {
    format!("{}#{}", account.server_url, account.device_id)
}

impl Engine {
    /// Opens the engine: creates the data directory, migrates the state database and
    /// loads the signed-in account's token.
    pub async fn open(config: EngineConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|e| Error::io(format!("creating {}", config.data_dir.display()), e))?;
        let data_dir = platform::canonical(&config.data_dir)
            .map_err(|e| Error::io(format!("reading {}", config.data_dir.display()), e))?;
        let db = Db::open(&data_dir.join("state.db"))?;
        let vault = config
            .token_vault
            .unwrap_or_else(|| TokenVault::system(&data_dir));
        let engine = Self {
            inner: Arc::new(Inner {
                cache_dir: data_dir.join("chunk-cache"),
                data_dir,
                db,
                vault,
                http: api::http_client()?,
                retry: config.retry,
                login_timeout: config.login_timeout,
                session: RwLock::new(None),
                locks: Mutex::new(HashMap::new()),
            }),
        };
        if let Some(account) = engine.inner.db.account()? {
            let inner = engine.inner.clone();
            let key = token_key(&account);
            let location = account.token_location;
            let token = tokio::task::spawn_blocking(move || inner.vault.load(&key, location))
                .await
                .map_err(|e| Error::TokenStore(format!("token task: {e}")))?;
            match token {
                Ok(Some(token)) => engine.set_session(account, token)?,
                Ok(None) => tracing::warn!("the device token is gone: sign in again"),
                Err(error) => tracing::warn!(%error, "could not read the device token"),
            }
        }
        Ok(engine)
    }

    /// The engine's data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub(crate) fn db(&self) -> &Db {
        &self.inner.db
    }

    fn set_session(&self, account: Account, token: Secret) -> Result<()> {
        let base = api::parse_server_url(&account.server_url)?;
        let api = Api::new(self.inner.http.clone(), base, Some(token), self.inner.retry);
        *self
            .inner
            .session
            .write()
            .map_err(|_| Error::State("session lock poisoned".to_owned()))? =
            Some(Session { account, api });
        Ok(())
    }

    /// The API client of the signed-in account.
    pub(crate) fn api(&self) -> Result<Api> {
        self.inner
            .session
            .read()
            .map_err(|_| Error::State("session lock poisoned".to_owned()))?
            .as_ref()
            .map(|s| s.api.clone())
            .ok_or(Error::NotSignedIn)
    }

    /// The signed-in account (stored), whether or not its token could be loaded.
    pub fn account(&self) -> Result<Option<Account>> {
        self.inner.db.account()
    }

    /// Whether a usable token is loaded.
    #[must_use]
    pub fn signed_in(&self) -> bool {
        self.inner.session.read().is_ok_and(|s| s.is_some())
    }

    /// Starts a browser sign-in: checks the server answers, binds the loopback
    /// listener. Open [`PendingLogin::browser_url`] in the browser, then call
    /// [`Engine::finish_login`].
    pub async fn begin_login(
        &self,
        server_url: &str,
        device_name: Option<&str>,
        ctx: &OpContext,
    ) -> Result<PendingLogin> {
        let base = api::parse_server_url(server_url)?;
        Api::new(
            self.inner.http.clone(),
            base.clone(),
            None,
            self.inner.retry,
        )
        .health(ctx)
        .await?;
        let name = device_name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(platform::device_name, str::to_owned);
        PendingLogin::start(base, &name, platform::os_name(), self.inner.login_timeout).await
    }

    /// Waits for the browser, exchanges the code for a device token, stores it and the
    /// account. Signing in to another server or account unlinks every folder (their
    /// server ids belong to the old one; local files stay).
    pub async fn finish_login(&self, pending: PendingLogin, ctx: &OpContext) -> Result<Account> {
        let callback = pending.wait(ctx).await?;
        let anonymous = Api::new(
            self.inner.http.clone(),
            pending.server.clone(),
            None,
            self.inner.retry,
        );
        let issued = anonymous
            .device_token(
                &DeviceTokenRequest {
                    code: Sensitive::new(callback.code.expose()),
                    code_verifier: Sensitive::new(pending.verifier.expose()),
                    redirect_uri: pending.redirect_uri.clone(),
                },
                ctx,
            )
            .await?;
        let token = Secret::new(issued.token.expose());
        let api = Api::new(
            self.inner.http.clone(),
            pending.server.clone(),
            Some(token.clone()),
            self.inner.retry,
        );
        let me = api.me(ctx).await?;
        let mut account = Account {
            server_url: api::server_url_text(&pending.server),
            user_id: me.id,
            email: me.email,
            display_name: me.display_name,
            device_id: issued.device_id,
            device_name: pending.device_name.clone(),
            token_location: crate::state::account::TokenLocation::Keyring,
            token_expires_at: issued.expires_at,
            signed_in_at: crate::state::now_rfc3339(),
        };
        let inner = self.inner.clone();
        let key = token_key(&account);
        let saved_token = token.clone();
        account.token_location =
            tokio::task::spawn_blocking(move || inner.vault.save(&key, &saved_token))
                .await
                .map_err(|e| Error::TokenStore(format!("token task: {e}")))??;

        let previous = self.inner.db.account()?;
        if let Some(previous) = &previous
            && token_key(previous) != token_key(&account)
        {
            let inner = self.inner.clone();
            let old_key = token_key(previous);
            let _ignored = tokio::task::spawn_blocking(move || inner.vault.delete(&old_key)).await;
        }
        if self.inner.db.meta(LAST_ACCOUNT)?.as_deref() != Some(account.link_key().as_str()) {
            let linked = self.inner.db.folders()?.len();
            if linked > 0 {
                tracing::info!(linked, "another account signed in: unlinking the folders");
            }
            self.inner.db.delete_all_folders()?;
            self.inner.db.set_meta(LAST_ACCOUNT, &account.link_key())?;
        }
        self.inner.db.set_account(&account)?;
        self.set_session(account.clone(), token)?;
        self.log(&NewActivity {
            action: "login",
            outcome: "ok",
            ..NewActivity::default()
        });
        Ok(account)
    }

    /// Forgets the token and the account on this device (the device stays listed on the
    /// server until revoked from the web UI). Folder links are kept for the next
    /// sign-in to the same account.
    pub async fn logout(&self) -> Result<()> {
        if let Some(account) = self.inner.db.account()? {
            let inner = self.inner.clone();
            let key = token_key(&account);
            tokio::task::spawn_blocking(move || inner.vault.delete(&key))
                .await
                .map_err(|e| Error::TokenStore(format!("token task: {e}")))??;
        }
        self.inner.db.clear_account()?;
        *self
            .inner
            .session
            .write()
            .map_err(|_| Error::State("session lock poisoned".to_owned()))? = None;
        self.log(&NewActivity {
            action: "logout",
            outcome: "ok",
            ..NewActivity::default()
        });
        Ok(())
    }

    /// Refreshes the cached profile (name, email) from the server.
    pub async fn refresh_profile(&self, ctx: &OpContext) -> Result<Account> {
        let me = self.api()?.me(ctx).await?;
        self.inner.db.update_profile(&me.email, &me.display_name)?;
        let account = self.inner.db.account()?.ok_or(Error::NotSignedIn)?;
        if let Ok(mut session) = self.inner.session.write()
            && let Some(session) = session.as_mut()
        {
            session.account = account.clone();
        }
        Ok(account)
    }

    /// The server's version (`GET /health`).
    pub async fn server_version(&self, ctx: &OpContext) -> Result<String> {
        Ok(self.api()?.health(ctx).await?.version)
    }

    /// The scopes the account can see.
    pub async fn scopes(&self, ctx: &OpContext) -> Result<Vec<ScopeInfo>> {
        self.api()?.scopes(ctx).await
    }

    /// The folders of a scope.
    pub async fn server_folders(&self, scope_id: &str, ctx: &OpContext) -> Result<Vec<FolderInfo>> {
        self.api()?.folders(scope_id, ctx).await
    }

    /// Creates a scope (owned by the account).
    pub async fn create_scope(&self, name: &str, ctx: &OpContext) -> Result<ScopeInfo> {
        self.api()?.create_scope(name, ctx).await
    }

    /// Creates a server folder in a scope (editor or more).
    pub async fn create_folder(
        &self,
        scope_id: &str,
        name: &str,
        emoji: Option<&str>,
        ctx: &OpContext,
    ) -> Result<FolderInfo> {
        self.api()?
            .create_folder(
                scope_id,
                &CreateFolderRequest {
                    name: name.to_owned(),
                    emoji: emoji.map(str::to_owned),
                    retention_max_versions: None,
                    retention_max_per_day: None,
                },
                ctx,
            )
            .await
    }

    /// The newest `limit` activity entries (optionally of one linked folder).
    pub fn activity(&self, folder: Option<i64>, limit: u32) -> Result<Vec<ActivityEntry>> {
        self.inner.db.activity(folder, limit)
    }

    /// Logs an activity entry (failures to log are only traced).
    pub(crate) fn log(&self, entry: &NewActivity<'_>) {
        if let Err(error) = self.inner.db.log_activity(entry) {
            tracing::warn!(%error, "could not record activity");
        }
    }

    /// The per-folder operation lock.
    pub(crate) async fn lock(&self, folder: i64) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let lock = self
            .inner
            .locks
            .lock()
            .map_err(|_| Error::State("lock table poisoned".to_owned()))?
            .entry(folder)
            .or_default()
            .clone();
        Ok(lock.lock_owned().await)
    }
}
