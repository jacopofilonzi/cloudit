//! Server configuration, read once from the environment at startup.
//!
//! Every variable is documented in `.env.example` and in `STRUCTURE.md`. Parsing goes
//! through [`Config::from_lookup`] so tests can feed values without touching the
//! process environment.

mod env;
mod secret;
mod storage;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

pub use secret::Secret;
pub use storage::{S3Config, StorageConfig};

use env::{Env, invalid};

/// Log output format (`LOG_FORMAT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, coloured when on a terminal.
    Pretty,
    /// One JSON object per line, for log collectors.
    Json,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// `BIND_ADDR`, default `0.0.0.0`.
    pub bind_addr: IpAddr,
    /// `PORT`, default `8080`.
    pub port: u16,
    /// `PUBLIC_URL`: externally visible base URL (no trailing slash), if known.
    pub public_url: Option<String>,
    /// `DATA_DIR`, default `./data`.
    pub data_dir: PathBuf,
    /// `WEB_DIR`, default `./apps/web/build`: the built web UI, served as an SPA.
    pub web_dir: PathBuf,
    /// Tracing filter: `RUST_LOG` if set, else `LOG_LEVEL`, default `info`.
    pub log_filter: String,
    /// `LOG_FORMAT`: `pretty` (default) or `json`.
    pub log_format: LogFormat,
    /// `DATABASE_URL` (`sqlite:…` or `postgres://…`; may hold a password). Unset: SQLite
    /// at `<DATA_DIR>/cloudit.db`.
    pub database_url: Option<Secret>,
    /// `DB_MAX_CONNECTIONS`: pool size; unset means the backend default (see `db`).
    pub db_max_connections: Option<u32>,
    /// `STORAGE` and friends.
    pub storage: StorageConfig,
    /// `REDIS_URL` (`redis://…`; may hold a password). Optional.
    pub redis_url: Option<Secret>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid value for {var}: {value:?} ({reason})")]
    Invalid {
        var: &'static str,
        value: String,
        reason: String,
    },
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the configuration through `lookup`; empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let env = Env(lookup);
        let get = |key: &str| env.get(key);

        let bind_addr = env.parse_or("BIND_ADDR", IpAddr::V4(Ipv4Addr::UNSPECIFIED))?;
        let port = env.parse_or("PORT", 8080)?;
        let public_url = match get("PUBLIC_URL") {
            None => None,
            Some(v) if v.starts_with("http://") || v.starts_with("https://") => {
                Some(v.trim_end_matches('/').to_owned())
            }
            Some(v) => {
                return Err(invalid(
                    "PUBLIC_URL",
                    &v,
                    "must start with http:// or https://",
                ));
            }
        };
        let log_format = match get("LOG_FORMAT").as_deref() {
            None | Some("pretty") => LogFormat::Pretty,
            Some("json") => LogFormat::Json,
            Some(other) => {
                return Err(invalid("LOG_FORMAT", other, "expected `pretty` or `json`"));
            }
        };
        let log_filter = get("RUST_LOG")
            .or_else(|| get("LOG_LEVEL"))
            .unwrap_or_else(|| "info".to_owned());

        let data_dir = get("DATA_DIR").map_or_else(|| PathBuf::from("data"), PathBuf::from);

        let database_url = match get("DATABASE_URL") {
            None => None,
            Some(v)
                if v.starts_with("sqlite:")
                    || v.starts_with("postgres://")
                    || v.starts_with("postgresql://") =>
            {
                Some(Secret::new(v))
            }
            // The value may contain a password: don't echo it.
            Some(_) => {
                return Err(invalid(
                    "DATABASE_URL",
                    "***",
                    "expected sqlite:… or postgres://…",
                ));
            }
        };
        let db_max_connections = match get("DB_MAX_CONNECTIONS") {
            None => None,
            Some(v) => match v.parse::<u32>() {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    return Err(invalid(
                        "DB_MAX_CONNECTIONS",
                        &v,
                        "expected a positive integer",
                    ));
                }
            },
        };
        let storage = StorageConfig::from_env(&env, &data_dir)?;
        let redis_url = match get("REDIS_URL") {
            None => None,
            Some(v)
                if v.starts_with("redis://")
                    || v.starts_with("redis+unix://")
                    || v.starts_with("unix://") =>
            {
                Some(Secret::new(v))
            }
            Some(_) => {
                return Err(invalid(
                    "REDIS_URL",
                    "***",
                    "expected redis://… (TLS `rediss://` is not supported)",
                ));
            }
        };

        Ok(Self {
            bind_addr,
            port,
            public_url,
            data_dir,
            web_dir: get("WEB_DIR")
                .map_or_else(|| ["apps", "web", "build"].iter().collect(), PathBuf::from),
            log_filter,
            log_format,
            database_url,
            db_max_connections,
            storage,
            redis_url,
        })
    }

    #[must_use]
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_addr, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let config = parse(&[]).unwrap();
        assert_eq!(config.socket_addr().to_string(), "0.0.0.0:8080");
        assert_eq!(config.public_url, None);
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(
            config.web_dir,
            PathBuf::from("apps").join("web").join("build")
        );
        assert_eq!(config.log_filter, "info");
        assert_eq!(config.log_format, LogFormat::Pretty);
    }

    #[test]
    fn reads_values() {
        let config = parse(&[
            ("BIND_ADDR", "127.0.0.1"),
            ("PORT", "9000"),
            ("PUBLIC_URL", "https://sync.example.com/"),
            ("DATA_DIR", "/data"),
            ("WEB_DIR", "/app/web"),
            ("LOG_LEVEL", "debug"),
            ("LOG_FORMAT", "json"),
        ])
        .unwrap();
        assert_eq!(config.socket_addr().to_string(), "127.0.0.1:9000");
        assert_eq!(
            config.public_url.as_deref(),
            Some("https://sync.example.com")
        );
        assert_eq!(config.data_dir, PathBuf::from("/data"));
        assert_eq!(config.web_dir, PathBuf::from("/app/web"));
        assert_eq!(config.log_filter, "debug");
        assert_eq!(config.log_format, LogFormat::Json);
    }

    #[test]
    fn rust_log_wins_over_log_level() {
        let config = parse(&[
            ("LOG_LEVEL", "debug"),
            ("RUST_LOG", "warn,cloudit_server=trace"),
        ])
        .unwrap();
        assert_eq!(config.log_filter, "warn,cloudit_server=trace");
    }

    #[test]
    fn empty_values_mean_default() {
        let config = parse(&[("PORT", " "), ("PUBLIC_URL", "")]).unwrap();
        assert_eq!(config.port, 8080);
        assert_eq!(config.public_url, None);
    }

    #[test]
    fn infrastructure_defaults() {
        let config = parse(&[("DATA_DIR", "/data")]).unwrap();
        assert!(config.database_url.is_none());
        assert_eq!(config.db_max_connections, None);
        assert!(config.redis_url.is_none());
        match config.storage {
            StorageConfig::Fs { dir } => assert_eq!(dir, PathBuf::from("/data").join("blobs")),
            StorageConfig::S3(_) => panic!("expected fs storage"),
        }
    }

    #[test]
    fn reads_s3_storage() {
        let config = parse(&[
            ("DATABASE_URL", "postgres://u:p@db/cloudit"),
            ("DB_MAX_CONNECTIONS", "20"),
            ("REDIS_URL", "redis://valkey:6379"),
            ("STORAGE", "s3"),
            ("S3_ENDPOINT", "http://minio:9000"),
            ("S3_BUCKET", "cloudit"),
            ("S3_ACCESS_KEY_ID", "key"),
            ("S3_SECRET_ACCESS_KEY", "secret"),
            ("S3_PRESIGN", "true"),
            ("S3_PUBLIC_ENDPOINT", "https://s3.example.com"),
            ("S3_PRESIGN_TTL", "1h"),
        ])
        .unwrap();
        assert_eq!(
            config.database_url.as_ref().map(Secret::expose),
            Some("postgres://u:p@db/cloudit")
        );
        assert_eq!(config.db_max_connections, Some(20));
        assert!(config.redis_url.is_some());
        let StorageConfig::S3(s3) = config.storage else {
            panic!("expected s3 storage");
        };
        assert_eq!(
            s3.endpoint.as_ref().map(url::Url::as_str),
            Some("http://minio:9000/")
        );
        assert_eq!(s3.region, "us-east-1");
        assert_eq!(s3.bucket, "cloudit");
        assert!(s3.force_path_style);
        assert!(s3.presign);
        assert_eq!(s3.presign_ttl, std::time::Duration::from_secs(3600));
        assert_eq!(
            s3.public_endpoint.as_ref().map(url::Url::host_str),
            Some(Some("s3.example.com"))
        );
        // Secrets never show up in Debug output.
        let debug = format!("{:?}", s3.secret_access_key);
        assert!(!debug.contains("secret"), "{debug}");
    }

    #[test]
    fn rejects_invalid_infrastructure_values() {
        assert!(parse(&[("DATABASE_URL", "mysql://x")]).is_err());
        assert!(parse(&[("DB_MAX_CONNECTIONS", "0")]).is_err());
        assert!(parse(&[("REDIS_URL", "rediss://x")]).is_err());
        assert!(parse(&[("STORAGE", "gcs")]).is_err());
        assert!(
            parse(&[("S3_PRESIGN", "true")]).is_err(),
            "presign needs s3"
        );
        assert!(parse(&[("STORAGE", "s3")]).is_err(), "bucket is required");
        let s3 = |extra: (&'static str, &'static str)| {
            parse(&[("STORAGE", "s3"), ("S3_BUCKET", "b"), extra])
        };
        assert!(s3(("S3_ENDPOINT", "minio:9000")).is_err());
        assert!(s3(("S3_PUBLIC_ENDPOINT", "ftp://x")).is_err());
        assert!(s3(("S3_ACCESS_KEY_ID", "only-the-key")).is_err());
        assert!(s3(("S3_PRESIGN_TTL", "8d")).is_err());
        assert!(s3(("S3_PRESIGN_TTL", "0")).is_err());
        assert!(s3(("S3_FORCE_PATH_STYLE", "maybe")).is_err());
        assert!(s3(("S3_REGION", "eu-west-1")).is_ok());
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(parse(&[("PORT", "eighty")]).is_err());
        assert!(parse(&[("PORT", "70000")]).is_err());
        assert!(parse(&[("BIND_ADDR", "localhost:1")]).is_err());
        assert!(parse(&[("PUBLIC_URL", "sync.example.com")]).is_err());
        assert!(parse(&[("LOG_FORMAT", "xml")]).is_err());
    }
}
