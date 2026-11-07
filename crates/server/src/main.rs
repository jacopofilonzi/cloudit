//! `cloudit-server` entry point: subcommands (see `cli`), or config, logging, state
//! (database, storage, Redis), router, serve.

use std::net::SocketAddr;

use anyhow::Context as _;
use cloudit_server::cli::{self, Command};
use cloudit_server::config::Config;
use cloudit_server::routes::router;
use cloudit_server::state::AppState;
use cloudit_server::{healthcheck, logging, shutdown};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = cli::parse(&args)?;
    if command == Command::Help {
        return cli::help();
    }

    let config = Config::from_env().context("reading configuration")?;
    match command {
        Command::Serve => serve(config).await,
        Command::Healthcheck => healthcheck::run(&config),
        Command::SetupToken => cli::setup_token(&config).await,
        Command::AdminResetPassword { email } => cli::reset_password(&config, &email).await,
        Command::Help => cli::help(),
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    logging::init(&config)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %config.data_dir.display(),
        web_dir = %config.web_dir.display(),
        public_url = config.public_url.as_deref().unwrap_or("-"),
        trust_proxy_headers = config.trust_proxy_headers,
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

    // The peer address is the client IP for rate limits (unless proxy headers are
    // trusted, see `auth::client`).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown::signal())
    .await
    .context("serving HTTP")?;

    tracing::info!("stopped");
    Ok(())
}
