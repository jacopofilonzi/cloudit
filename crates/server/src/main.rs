//! `cloudit-server` entry point: config, logging, router, serve.

use std::sync::Arc;

use anyhow::Context as _;
use cloudit_server::config::Config;
use cloudit_server::routes::{AppState, router};
use cloudit_server::{healthcheck, logging, shutdown};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("reading configuration")?;

    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        return healthcheck::run(&config);
    }

    logging::init(&config)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %config.data_dir.display(),
        web_dir = %config.web_dir.display(),
        public_url = config.public_url.as_deref().unwrap_or("-"),
        "starting cloudit-server"
    );

    let addr = config.socket_addr();
    let app = router(AppState {
        config: Arc::new(config),
    });

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown::signal())
        .await
        .context("serving HTTP")?;

    tracing::info!("stopped");
    Ok(())
}
