//! Browser sign-in of a device: RFC 8252 loopback redirect + PKCE S256 (RFC 7636).
//!
//! [`PendingLogin::start`] binds a one-shot HTTP listener on `127.0.0.1:<random port>`
//! and builds the URL of the web UI's `/device` page (`code_challenge`,
//! `code_challenge_method=S256`, `redirect_uri=http://127.0.0.1:<port>/callback`,
//! `state`, `device_name`, `os`). The caller opens it in the browser (the Tauri app with
//! its opener plugin, a CLI by printing it, or [`crate::platform::open_browser`]). The
//! user signs in and allows the device there; the browser comes back to the listener
//! with `code` + `state` (or `error=access_denied` + `state` on "Cancel").
//! [`PendingLogin::wait`] answers it with a small "you can close this tab" page and
//! returns the code; requests with another `state` (a stale tab) are answered with an
//! error page and ignored. The engine then exchanges the code and the verifier at
//! `POST /devices/token`.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::error::{Error, LoginFailure, Result};
use crate::progress::OpContext;
use crate::secret::Secret;

/// Default time the user has to finish in the browser.
pub const DEFAULT_LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// Largest request head read from the browser.
const MAX_REQUEST: usize = 16 * 1024;
/// Time a browser connection has to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A started browser sign-in, waiting for the browser to come back.
#[derive(Debug)]
pub struct PendingLogin {
    browser_url: String,
    pub(crate) server: Url,
    pub(crate) device_name: String,
    pub(crate) verifier: Secret,
    state: String,
    pub(crate) redirect_uri: String,
    listener: TcpListener,
    timeout: Duration,
}

/// The browser's answer: the authorization code.
#[derive(Debug)]
pub(crate) struct Callback {
    pub code: Secret,
}

fn random_b64(len: usize) -> Result<String> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|e| Error::Protocol(format!("no randomness: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

impl PendingLogin {
    /// Binds the loopback listener and builds the browser URL for `server` (from
    /// [`crate::api::parse_server_url`]).
    pub async fn start(
        server: Url,
        device_name: &str,
        os: &str,
        timeout: Duration,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| Error::io("starting the sign-in listener", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::io("starting the sign-in listener", e))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        // 32 random bytes → a 43-character verifier (RFC 7636: 43..=128).
        let verifier = Secret::new(random_b64(32)?);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes()));
        let state = random_b64(16)?;
        let mut url = server
            .join("device")
            .map_err(|e| Error::InvalidInput(format!("server URL: {e}")))?;
        url.query_pairs_mut()
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &state)
            .append_pair("device_name", device_name)
            .append_pair("os", os);
        Ok(Self {
            browser_url: url.into(),
            server,
            device_name: device_name.to_owned(),
            verifier,
            state,
            redirect_uri,
            listener,
            timeout,
        })
    }

    /// The URL to open in the browser.
    #[must_use]
    pub fn browser_url(&self) -> &str {
        &self.browser_url
    }

    /// The loopback redirect URI.
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Waits for the browser (until the timeout or cancellation).
    pub(crate) async fn wait(&self, ctx: &OpContext) -> Result<Callback> {
        let accept = async {
            loop {
                let (stream, _) = self
                    .listener
                    .accept()
                    .await
                    .map_err(|e| Error::io("waiting for the browser", e))?;
                match self.handle(stream).await {
                    Ok(Some(outcome)) => return outcome,
                    Ok(None) => {}
                    Err(error) => tracing::debug!(%error, "bad sign-in callback request"),
                }
            }
        };
        ctx.run(async {
            tokio::time::timeout(self.timeout, accept)
                .await
                .unwrap_or(Err(Error::Login(LoginFailure::TimedOut)))
        })
        .await
    }

    /// Answers one browser request; `Some` when it ends the sign-in.
    async fn handle(&self, mut stream: TcpStream) -> std::io::Result<Option<Result<Callback>>> {
        let target = match tokio::time::timeout(REQUEST_TIMEOUT, read_target(&mut stream)).await {
            Ok(target) => target?,
            Err(_) => return Ok(None),
        };
        let Some(target) = target else {
            respond(&mut stream, 400, PAGE_BAD).await?;
            return Ok(None);
        };
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, 400, PAGE_BAD).await?;
            return Ok(None);
        };
        if url.path() != "/callback" {
            respond(&mut stream, 404, PAGE_NOT_FOUND).await?;
            return Ok(None);
        }
        let mut state = None;
        let mut code = None;
        let mut error = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "state" => state = Some(value.into_owned()),
                "code" => code = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }
        if state.as_deref() != Some(self.state.as_str()) {
            // A stale tab or another app's sign-in: not ours.
            respond(&mut stream, 400, PAGE_BAD).await?;
            return Ok(None);
        }
        if error.is_some() {
            respond(&mut stream, 200, PAGE_CANCELLED).await?;
            return Ok(Some(Err(Error::Login(LoginFailure::Denied))));
        }
        match code.filter(|c| !c.is_empty()) {
            Some(code) => {
                respond(&mut stream, 200, PAGE_DONE).await?;
                Ok(Some(Ok(Callback {
                    code: Secret::new(code),
                })))
            }
            None => {
                respond(&mut stream, 400, PAGE_BAD).await?;
                Ok(Some(Err(Error::Login(LoginFailure::BadCallback))))
            }
        }
    }
}

/// Reads the request head; the request target of a `GET`, `None` if malformed.
async fn read_target(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await?;
        if n == 0 || buf.len() + n > MAX_REQUEST {
            return Ok(None);
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next().unwrap_or_default().split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Ok(Some(target.to_owned())),
        _ => Ok(None),
    }
}

async fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let page = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>cloudit</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;\
         padding:0 1rem;line-height:1.5}}</style></head><body>{body}</body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        page.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(page.as_bytes()).await?;
    stream.shutdown().await
}

const PAGE_DONE: &str = "<h1>cloudit is signed in</h1><p>You can close this tab and go back \
    to the app.</p><p lang=\"it\">Accesso eseguito: puoi chiudere questa scheda e tornare \
    all'app.</p>";
const PAGE_CANCELLED: &str = "<h1>Sign-in cancelled</h1><p>You can close this tab.</p>\
    <p lang=\"it\">Accesso annullato: puoi chiudere questa scheda.</p>";
const PAGE_BAD: &str = "<h1>This link is not valid</h1><p>Start the sign-in again from the \
    app.</p><p lang=\"it\">Link non valido: avvia di nuovo l'accesso dall'app.</p>";
const PAGE_NOT_FOUND: &str = "<h1>Not found</h1>";

#[cfg(test)]
mod tests {
    use super::*;

    async fn pending(timeout: Duration) -> PendingLogin {
        let server = crate::api::parse_server_url("http://127.0.0.1:9/").unwrap();
        PendingLogin::start(server, "My PC", "windows", timeout)
            .await
            .unwrap()
    }

    fn query(url: &str, key: &str) -> String {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == key)
            .unwrap()
            .1
            .into_owned()
    }

    /// A fake browser request; the status line and body of the answer.
    async fn browse(redirect_uri: &str, query: &str) -> String {
        let url = Url::parse(redirect_uri).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
            .await
            .unwrap();
        let request = format!(
            "GET {}?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            url.path()
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        answer
    }

    #[tokio::test]
    async fn browser_url_has_the_device_contract() {
        let login = pending(DEFAULT_LOGIN_TIMEOUT).await;
        let url = login.browser_url();
        assert!(url.starts_with("http://127.0.0.1:9/device?"));
        let challenge = query(url, "code_challenge");
        assert_eq!(challenge.len(), 43);
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.expose().as_bytes()))
        );
        assert_eq!(query(url, "code_challenge_method"), "S256");
        assert_eq!(query(url, "redirect_uri"), login.redirect_uri());
        assert!(login.redirect_uri().starts_with("http://127.0.0.1:"));
        assert!(login.redirect_uri().ends_with("/callback"));
        assert_eq!(query(url, "device_name"), "My PC");
        assert_eq!(query(url, "os"), "windows");
        assert!(!query(url, "state").is_empty());
    }

    #[tokio::test]
    async fn returns_the_code_and_ignores_other_states() {
        let login = pending(DEFAULT_LOGIN_TIMEOUT).await;
        let state = query(login.browser_url(), "state");
        let redirect = login.redirect_uri().to_owned();
        let browser = tokio::spawn(async move {
            let other = browse(&redirect, "code=evil&state=other").await;
            assert!(other.starts_with("HTTP/1.1 400"), "{other}");
            let lost = browse(&redirect.replace("/callback", "/favicon.ico"), "").await;
            assert!(lost.starts_with("HTTP/1.1 404"), "{lost}");
            let done = browse(&redirect, &format!("code=the-code&state={state}")).await;
            assert!(done.starts_with("HTTP/1.1 200"), "{done}");
            assert!(done.contains("close this tab"));
        });
        let callback = login.wait(&OpContext::new()).await.unwrap();
        assert_eq!(callback.code.expose(), "the-code");
        browser.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_in_the_browser_is_denied() {
        let login = pending(DEFAULT_LOGIN_TIMEOUT).await;
        let state = query(login.browser_url(), "state");
        let redirect = login.redirect_uri().to_owned();
        let browser = tokio::spawn(async move {
            browse(&redirect, &format!("error=access_denied&state={state}")).await
        });
        let err = login.wait(&OpContext::new()).await.unwrap_err();
        assert!(matches!(err, Error::Login(LoginFailure::Denied)), "{err:?}");
        assert!(browser.await.unwrap().contains("cancelled"));
    }

    #[tokio::test]
    async fn times_out_and_cancels() {
        let login = pending(Duration::from_millis(50)).await;
        let err = login.wait(&OpContext::new()).await.unwrap_err();
        assert!(
            matches!(err, Error::Login(LoginFailure::TimedOut)),
            "{err:?}"
        );

        let login = pending(DEFAULT_LOGIN_TIMEOUT).await;
        let ctx = OpContext::new();
        ctx.cancel_token().cancel();
        let err = login.wait(&ctx).await.unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err:?}");
    }
}
