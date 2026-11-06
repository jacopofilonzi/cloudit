//! Tracing subscriber setup.

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

/// Installs the global tracing subscriber. Call once, at startup.
pub fn init(config: &Config) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(&config.log_filter)
        .with_context(|| format!("invalid log filter {:?}", config.log_filter))?;
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let result = match config.log_format {
        LogFormat::Pretty => builder.try_init(),
        LogFormat::Json => builder.json().try_init(),
    };
    result
        .map_err(|e| anyhow::anyhow!(e))
        .context("installing the tracing subscriber")
}
