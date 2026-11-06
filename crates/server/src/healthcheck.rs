//! `cloudit-server healthcheck`: probes `GET /health` on the local instance and
//! exits 0 if it answers 200. Used by the Docker `HEALTHCHECK`, so the runtime
//! image needs no curl/wget. Plain HTTP/1.0 over std, no extra dependencies.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use anyhow::{Context as _, bail};

use crate::config::Config;

const TIMEOUT: Duration = Duration::from_secs(3);

/// Runs the probe against `127.0.0.1:$PORT`.
pub fn run(config: &Config) -> anyhow::Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let mut stream =
        TcpStream::connect_timeout(&addr, TIMEOUT).with_context(|| format!("connecting {addr}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .context("setting read timeout")?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .context("setting write timeout")?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .context("sending request")?;

    // "HTTP/1.x 200" is 12 bytes; read until we have them or the server closes.
    let mut head = Vec::with_capacity(16);
    stream
        .take(16)
        .read_to_end(&mut head)
        .context("reading response")?;
    let status_line = String::from_utf8_lossy(&head);
    if status_line.starts_with("HTTP/1.0 200") || status_line.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        bail!("unhealthy response: {status_line:?}")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::run;
    use crate::config::Config;
    use crate::routes::{AppState, router};

    fn config_for_port(port: u16) -> Config {
        Config::from_lookup(|key| match key {
            "PORT" => Some(port.to_string()),
            "WEB_DIR" => Some("does-not-exist".to_owned()),
            _ => None,
        })
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn succeeds_against_a_running_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = config_for_port(port);
        let app = router(AppState {
            config: Arc::new(config.clone()),
        });
        tokio::spawn(async move { axum::serve(listener, app).await });

        let result = tokio::task::spawn_blocking(move || run(&config))
            .await
            .unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_when_nothing_listens() {
        // Bind then drop to get a port that is (almost certainly) closed.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let config = config_for_port(port);
        let result = tokio::task::spawn_blocking(move || run(&config))
            .await
            .unwrap();
        assert!(result.is_err());
    }
}
