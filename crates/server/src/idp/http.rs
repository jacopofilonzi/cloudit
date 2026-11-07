//! The one outgoing HTTP client for identity providers.
//!
//! - rustls (reqwest's default provider and the platform's trust store);
//! - 5 s connect and 15 s total timeout per request;
//! - **redirects are never followed**: discovery, keys, token and user-info endpoints
//!   are configured URLs, and following redirects would let a provider point the server
//!   at internal addresses (SSRF);
//! - response bodies are capped at [`MAX_BODY`].
//!
//! [`HttpClient::adapter`] exposes it to `openidconnect`, which brings no HTTP client
//! of its own here (its `reqwest` feature is off).

use std::future::Future;
use std::time::Duration;

use anyhow::Context as _;
use openidconnect::{HttpRequest, HttpResponse};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue, USER_AGENT};
use serde::de::DeserializeOwned;

/// Largest response body read from a provider.
pub const MAX_BODY: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(15);

/// A failed request to a provider (the message never contains secrets: request
/// bodies and headers are not included).
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("request to {url} failed: {message}")]
    Request { url: String, message: String },
    #[error("response from {url} is larger than {MAX_BODY} bytes")]
    TooLarge { url: String },
    #[error("unexpected status {status} from {url}")]
    Status { url: String, status: u16 },
    #[error("invalid JSON from {url}: {message}")]
    Json { url: String, message: String },
}

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpClient")
    }
}

/// `scheme://host[:port]/path` of a URL, without query (codes, tokens) for messages.
fn redact(url: &reqwest::Url) -> String {
    let mut url = url.clone();
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_password(None);
    let _ = url.set_username("");
    url.to_string()
}

impl HttpClient {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building the identity-provider HTTP client")?;
        Ok(Self { client })
    }

    /// Sends a request and reads the whole (bounded) body.
    pub async fn send(&self, mut request: reqwest::Request) -> Result<HttpResponse, HttpError> {
        let url = redact(request.url());
        request.headers_mut().insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("cloudit-server/", env!("CARGO_PKG_VERSION"))),
        );
        let fail = |message: String| HttpError::Request {
            url: url.clone(),
            message,
        };
        let mut response = self
            .client
            .execute(request)
            .await
            .map_err(|e| fail(e.without_url().to_string()))?;
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
        {
            return Err(HttpError::TooLarge { url });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| fail(e.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(HttpError::TooLarge { url });
            }
            body.extend_from_slice(&chunk);
        }
        let mut out =
            http_response(response.status().as_u16(), body).map_err(|e| fail(format!("{e:#}")))?;
        *out.headers_mut() = response.headers().clone();
        Ok(out)
    }

    /// Sends an `openidconnect` request.
    pub async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let url = request.uri().to_string();
        let request = reqwest::Request::try_from(request).map_err(|e| HttpError::Request {
            url,
            message: e.without_url().to_string(),
        })?;
        self.send(request).await
    }

    /// An `openidconnect::AsyncHttpClient` (any `Fn(HttpRequest) -> Future`).
    pub fn adapter(
        &self,
    ) -> impl Fn(
        HttpRequest,
    )
        -> std::pin::Pin<Box<dyn Future<Output = Result<HttpResponse, HttpError>> + Send>>
    + use<> {
        let client = self.clone();
        move |request| {
            let client = client.clone();
            Box::pin(async move { client.execute(request).await })
        }
    }

    /// `GET url` with `Accept: application/json` (and a bearer token), parsed as `T`.
    /// Non-2xx answers are errors.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &reqwest::Url,
        bearer: Option<&str>,
    ) -> Result<T, HttpError> {
        let mut request = reqwest::Request::new(reqwest::Method::GET, url.clone());
        let headers = request.headers_mut();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(token) = bearer {
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                HttpError::Request {
                    url: redact(url),
                    message: "invalid access token characters".to_owned(),
                }
            })?;
            headers.insert(AUTHORIZATION, value);
        }
        let response = self.send(request).await?;
        json_body(&redact(url), &response)
    }

    /// `POST url` with a form body, `Accept: application/json` and optionally
    /// `Authorization: Basic <basic_auth>` (already encoded); returns the raw
    /// response whatever its status (token endpoints answer errors as JSON).
    pub async fn post_form_auth(
        &self,
        url: &reqwest::Url,
        form: &[(&str, &str)],
        basic_auth: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let mut builder = self
            .client
            .post(url.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
        if let Some(credentials) = basic_auth {
            builder = builder.header(AUTHORIZATION, format!("Basic {credentials}"));
        }
        let request = builder.build().map_err(|e| HttpError::Request {
            url: redact(url),
            message: e.without_url().to_string(),
        })?;
        self.send(request).await
    }
}

fn http_response(status: u16, body: Vec<u8>) -> anyhow::Result<HttpResponse> {
    openidconnect::http::Response::builder()
        .status(status)
        .body(body)
        .context("building a response")
}

/// Parses a 2xx JSON body.
pub fn json_body<T: DeserializeOwned>(url: &str, response: &HttpResponse) -> Result<T, HttpError> {
    if !response.status().is_success() {
        return Err(HttpError::Status {
            url: url.to_owned(),
            status: response.status().as_u16(),
        });
    }
    serde_json::from_slice(response.body()).map_err(|e| HttpError::Json {
        url: url.to_owned(),
        message: e.to_string(),
    })
}
