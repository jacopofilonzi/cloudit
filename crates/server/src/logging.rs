//! Tracing subscriber setup.

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

/// Installs the global tracing subscriber. Call once, at startup.
pub fn init(config: &Config) -> anyhow::Result<()> {
    let mut filter = EnvFilter::try_new(&config.log_filter)
        .with_context(|| format!("invalid log filter {:?}", config.log_filter))?;
    // PostgreSQL NOTICEs ("relation … already exists, skipping" from idempotent DDL)
    // arrive at info level and are noise; an explicit `sqlx` directive still wins.
    if !config.log_filter.contains("sqlx") {
        filter = filter.add_directive(
            "sqlx::postgres::notice=warn"
                .parse()
                .context("parsing the built-in log directive")?,
        );
    }
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let result = match config.log_format {
        LogFormat::Pretty => builder.try_init(),
        LogFormat::Json => builder.json().try_init(),
    };
    result
        .map_err(|e| anyhow::anyhow!(e))
        .context("installing the tracing subscriber")
}
