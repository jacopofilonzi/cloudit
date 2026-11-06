//! Server configuration, read once from the environment at startup.
//!
//! Every variable is documented in `.env.example` and in `STRUCTURE.md`. Parsing goes
//! through [`Config::from_lookup`] so tests can feed values without touching the
//! process environment.

mod secret;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

pub use secret::Secret;

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

fn invalid(var: &'static str, value: &str, reason: impl ToString) -> ConfigError {
    ConfigError::Invalid {
        var,
        value: value.to_owned(),
        reason: reason.to_string(),
    }
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the configuration through `lookup`; empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };

        let bind_addr = match get("BIND_ADDR") {
            None => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            Some(v) => v.parse().map_err(|e| invalid("BIND_ADDR", &v, e))?,
        };
        let port = match get("PORT") {
            None => 8080,
            Some(v) => v.parse().map_err(|e| invalid("PORT", &v, e))?,
        };
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

        Ok(Self {
            bind_addr,
            port,
            public_url,
            data_dir: get("DATA_DIR").map_or_else(|| PathBuf::from("data"), PathBuf::from),
            web_dir: get("WEB_DIR")
                .map_or_else(|| ["apps", "web", "build"].iter().collect(), PathBuf::from),
            log_filter,
            log_format,
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
    fn rejects_invalid_values() {
        assert!(parse(&[("PORT", "eighty")]).is_err());
        assert!(parse(&[("PORT", "70000")]).is_err());
        assert!(parse(&[("BIND_ADDR", "localhost:1")]).is_err());
        assert!(parse(&[("PUBLIC_URL", "sync.example.com")]).is_err());
        assert!(parse(&[("LOG_FORMAT", "xml")]).is_err());
    }
}
