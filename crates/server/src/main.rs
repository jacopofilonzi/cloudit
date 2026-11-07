//! `cloudit-server` entry point: config, logging, state (database, storage, Redis),
//! router, serve.

use anyhow::Context as _;
use cloudit_server::config::Config;
use cloudit_server::routes::router;
use cloudit_server::state::AppState;
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
    let state = AppState::init(config).await?;
    tracing::info!(instance_id = %state.instance_id, "ready to serve");
    let app = router(state);

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
