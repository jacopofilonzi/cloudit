//! [`AppState`]: the shared handles every request needs, built once at startup.
//!
//! Nothing correctness-relevant may live here in memory: instances must stay
//! interchangeable (HA). Two exceptions, harmless by design: the rate limiter's
//! in-memory fallback (it only weakens abuse protection) and `setup_done`, a cache of a
//! fact that can never revert (setup, once completed, stays completed).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Context as _;
use sea_orm::DatabaseConnection;

use crate::accounts::setup;
use crate::cache::Redis;
use crate::cache::rate_limit::RateLimiter;
use crate::config::Config;
use crate::db;
use crate::db::lease::Leases;
use crate::storage::BlobStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: DatabaseConnection,
    pub blobs: BlobStore,
    /// `None` when `REDIS_URL` is unset (or invalid: then logged at startup).
    pub redis: Option<Redis>,
    pub rate_limiter: RateLimiter,
    /// Random per process start; identifies this instance as a lease holder and in logs.
    pub instance_id: Arc<str>,
    pub leases: Leases,
    /// Set once first-run setup is seen completed (see `auth::middleware`).
    pub setup_done: Arc<AtomicBool>,
}

impl AppState {
    /// Connects to the database and applies migrations, opens blob storage and prepares
    /// the optional Redis connection. Database and storage errors abort startup; Redis
    /// never does.
    pub async fn init(config: Config) -> anyhow::Result<Self> {
        let instance_id: Arc<str> = uuid::Uuid::new_v4().simple().to_string().into();

        let db = db::connect(&config).await?;
        tracing::info!(
            backend = db::backend_name(db.get_database_backend()),
            "database connected"
        );
        db::migrate::run(&db, &config.data_dir.join("backups"))
            .await
            .context("migrating the database")?;

        let setup_status = setup::init(&db, config.setup_token.as_ref())
            .await
            .context("preparing first-run setup")?;

        let blobs = BlobStore::from_config(&config.storage)?;
        tracing::info!(
            storage = %blobs.describe(),
            presign = blobs.presign_enabled(),
            "blob storage configured"
        );

        let redis = match &config.redis_url {
            None => None,
            Some(url) => match Redis::connect(url) {
                Ok(redis) => {
                    tracing::info!("Redis configured (connects lazily)");
                    Some(redis)
                }
                Err(err) => {
                    tracing::warn!(error = %format!("{err:#}"), "Redis disabled");
                    None
                }
            },
        };

        let leases = Leases::new(db.clone(), instance_id.to_string());
        Ok(Self {
            config: Arc::new(config),
            db,
            blobs,
            rate_limiter: RateLimiter::new(redis.clone()),
            redis,
            instance_id,
            leases,
            setup_done: Arc::new(AtomicBool::new(!setup_status.is_pending())),
        })
    }
}
