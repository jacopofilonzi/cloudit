//! HTTP client of the cloudit API (`/api/v1`).
//!
//! One shared `reqwest` client (rustls, no redirects, 10 s connect timeout, 90 s read
//! timeout so a dead event stream is noticed; each JSON call also has a total timeout).
//! Requests carry the device token as `Authorization: Bearer` (bearer requests need no
//! CSRF `Origin`). Error answers become [`Error`]s through the API's `ApiErrorBody`
//! (stable [`cloudit_core::protocol::ErrorCode`] plus `head_snapshot_id` / `hashes`).
//!
//! Retries: network errors (connect, timeout, broken connection) and the statuses 429,
//! 502, 503 and 504 are retried with exponential backoff, honouring `Retry-After`
//! (capped at 60 s); other 4xx / 5xx answers are returned at once. Every API call is
//! idempotent or safe to repeat (a repeated commit at worst answers `conflict` against
//! its own snapshot, which the push recognises).
//!
//! Chunk transfers ([`cloudit_core::protocol::Transfer`]): `proxy` URLs get the bearer
//! token (only when they point at the server's own origin), `presigned` URLs never get
//! any credential; relative URLs (`/api/v1/…`) are resolved against the server URL.

mod endpoints;

pub use endpoints::{CommitResult, FetchedManifest};

use std::time::Duration;

use bytes::Bytes;
use cloudit_core::protocol::{ApiErrorBody, ErrorCode};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use url::Url;

use crate::error::{Error, Result};
use crate::progress::OpContext;
use crate::secret::Secret;

/// How failed requests are retried.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// First backoff; doubled at each retry.
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
        }
    }
}

/// Longest `Retry-After` honoured.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Total timeout of a JSON call.
const JSON_TIMEOUT: Duration = Duration::from_secs(60);
/// Total timeout of a chunk transfer or manifest upload / download.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(300);
/// Largest JSON answer read.
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;

/// Builds the shared HTTP client.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("cloudit-client/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Network(format!("building the HTTP client: {e}")))
}

/// Parses and normalizes a server URL: `http(s)://host[:port][/path]`, no query,
/// fragment or credentials; returned with a trailing `/` (for joins).
pub fn parse_server_url(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    let mut url =
        Url::parse(raw).map_err(|e| Error::InvalidInput(format!("server URL {raw:?}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(Error::InvalidInput(format!(
            "server URL {raw:?}: only http(s)://host URLs"
        )));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(Error::InvalidInput(format!(
            "server URL {raw:?}: no query, fragment or credentials"
        )));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// The display / stored form of a server URL (no trailing `/`).
#[must_use]
pub fn server_url_text(url: &Url) -> String {
    url.as_str().trim_end_matches('/').to_owned()
}

/// A request body, rebuilt for each attempt.
#[derive(Debug, Clone)]
pub(crate) enum Body {
    Empty,
    Json(Vec<u8>),
    /// Raw bytes; the content type (if any) comes from the headers.
    Raw(Bytes),
}

/// One API request.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    method: Method,
    url: Url,
    body: Body,
    headers: Vec<(String, String)>,
    bearer: bool,
    timeout: Duration,
}

impl Call {
    fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            body: Body::Empty,
            headers: Vec::new(),
            bearer: true,
            timeout: JSON_TIMEOUT,
        }
    }

    fn json(mut self, body: &impl serde::Serialize) -> Result<Self> {
        self.body = Body::Json(
            serde_json::to_vec(body)
                .map_err(|e| Error::Protocol(format!("encoding a request: {e}")))?,
        );
        Ok(self)
    }

    fn raw(mut self, bytes: Bytes) -> Self {
        self.body = Body::Raw(bytes);
        self
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn anonymous(mut self) -> Self {
        self.bearer = false;
        self
    }

    fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// The API client of one server (and, once signed in, one device token).
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: Url,
    token: Option<Secret>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base.as_str())
            .field("token", &self.token)
            .finish()
    }
}

impl Api {
    /// A client for `base` (from [`parse_server_url`]).
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        base: Url,
        token: Option<Secret>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            http,
            base,
            token,
            retry,
        }
    }

    /// The server's base URL (with a trailing `/`).
    #[must_use]
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// `<base>api/v1/<path>`.
    fn url(&self, path: &str) -> Result<Url> {
        self.base
            .join(&format!("api/v1/{path}"))
            .map_err(|e| Error::Protocol(format!("building a URL for {path}: {e}")))
    }

    /// A transfer URL: absolute, or a server path resolved against the base.
    fn transfer_url(&self, raw: &str) -> Result<Url> {
        let url = if raw.starts_with("http://") || raw.starts_with("https://") {
            Url::parse(raw)
        } else {
            self.base.join(raw.trim_start_matches('/'))
        };
        url.map_err(|e| Error::Protocol(format!("transfer URL {raw:?}: {e}")))
    }

    /// Sends `call`, retrying transient failures (see the module docs).
    async fn send(&self, call: &Call, ctx: &OpContext) -> Result<Response> {
        let mut attempt = 0u32;
        loop {
            let mut request = self
                .http
                .request(call.method.clone(), call.url.clone())
                .timeout(call.timeout);
            if call.bearer
                && let Some(token) = &self.token
            {
                request = request.header(AUTHORIZATION, format!("Bearer {}", token.expose()));
            }
            for (name, value) in &call.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            request = match &call.body {
                Body::Empty => request,
                Body::Json(bytes) => request
                    .header(CONTENT_TYPE, "application/json")
                    .body(bytes.clone()),
                Body::Raw(bytes) => request.body(bytes.clone()),
            };
            let sent = ctx.run(async { Ok(request.send().await) }).await?;
            let wait = match sent {
                Ok(response) => {
                    let status = response.status();
                    if attempt < self.retry.max_retries && retryable(status) {
                        retry_after(response.headers()).unwrap_or_else(|| self.backoff(attempt))
                    } else {
                        return Ok(response);
                    }
                }
                Err(error) => {
                    if attempt < self.retry.max_retries && !error.is_builder() {
                        tracing::debug!(%error, url = %redacted(&call.url), "request failed, retrying");
                        self.backoff(attempt)
                    } else {
                        return Err(Error::Network(format!(
                            "{} {}: {error}",
                            call.method,
                            redacted(&call.url)
                        )));
                    }
                }
            };
            attempt += 1;
            ctx.run(async {
                tokio::time::sleep(wait).await;
                Ok(())
            })
            .await?;
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.retry
            .base_delay
            .saturating_mul(2u32.saturating_pow(attempt))
    }

    /// Sends `call` and decodes a JSON success answer.
    async fn call_json<T: DeserializeOwned>(&self, call: &Call, ctx: &OpContext) -> Result<T> {
        let response = self.send(call, ctx).await?;
        if !response.status().is_success() {
            return Err(error_of(response).await);
        }
        let bytes = read_bounded(response, MAX_JSON_BYTES).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::Protocol(format!("{} {}: {e}", call.method, call.url.path())))
    }

    /// Sends `call`; success answers have no body of interest.
    async fn call_empty(&self, call: &Call, ctx: &OpContext) -> Result<()> {
        let response = self.send(call, ctx).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(error_of(response).await)
        }
    }
}

fn retryable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER))
}

/// The URL without its query (presigned URLs carry signatures there).
fn redacted(url: &Url) -> String {
    let mut url = url.clone();
    url.set_query(None);
    url.to_string()
}

/// The error of a non-success answer.
async fn error_of(response: Response) -> Error {
    let status = response.status();
    let body = read_bounded(response, 64 * 1024).await.unwrap_or_default();
    match serde_json::from_slice::<ApiErrorBody>(&body) {
        Ok(body) => Error::from_api(status.as_u16(), body),
        Err(_) => {
            let code = match status {
                StatusCode::UNAUTHORIZED => ErrorCode::Unauthorized,
                StatusCode::FORBIDDEN => ErrorCode::Forbidden,
                StatusCode::NOT_FOUND => ErrorCode::NotFound,
                StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
                StatusCode::PAYLOAD_TOO_LARGE => ErrorCode::PayloadTooLarge,
                s if s.is_server_error() => ErrorCode::Unavailable,
                _ => ErrorCode::BadRequest,
            };
            Error::from_api(
                status.as_u16(),
                ApiErrorBody::new(code, format!("HTTP {status} without an API error body")),
            )
        }
    }
}

/// Reads a body of at most `max` bytes.
async fn read_bounded(mut response: Response, max: usize) -> Result<Vec<u8>> {
    let declared = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|len| len > max) {
        return Err(Error::Protocol(format!("answer larger than {max} bytes")));
    }
    let mut out = Vec::with_capacity(declared.unwrap_or(0).min(max));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Error::Network(format!("reading an answer: {e}")))?
    {
        if out.len() + chunk.len() > max {
            return Err(Error::Protocol(format!("answer larger than {max} bytes")));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_normalized() {
        assert_eq!(
            parse_server_url("https://sync.example.com")
                .unwrap()
                .as_str(),
            "https://sync.example.com/"
        );
        assert_eq!(
            parse_server_url(" http://127.0.0.1:8080/cloudit ")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8080/cloudit/"
        );
        for bad in [
            "ftp://x",
            "https://x/?a=1",
            "https://u:p@x",
            "not a url",
            "https://x/#f",
        ] {
            assert!(parse_server_url(bad).is_err(), "{bad}");
        }
        let url = parse_server_url("https://x/a/").unwrap();
        assert_eq!(server_url_text(&url), "https://x/a");
    }

    #[test]
    fn transfer_urls_resolve() {
        let base = parse_server_url("http://h:1/prefix").unwrap();
        let api = Api::new(reqwest::Client::new(), base, None, RetryPolicy::default());
        assert_eq!(
            api.transfer_url("/api/v1/folders/x/chunks/y")
                .unwrap()
                .as_str(),
            "http://h:1/prefix/api/v1/folders/x/chunks/y"
        );
        assert_eq!(
            api.transfer_url("https://s3.example/b/k?sig=1")
                .unwrap()
                .as_str(),
            "https://s3.example/b/k?sig=1"
        );
        assert_eq!(
            redacted(&Url::parse("https://s3/b?X-Amz-Signature=1").unwrap()),
            "https://s3/b"
        );
    }

    #[test]
    fn retry_after_is_capped() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "5".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(5)));
        headers.insert(RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(MAX_RETRY_AFTER));
    }
}
