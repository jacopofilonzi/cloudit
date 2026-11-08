//! A tiny API client for the seed tool: JSON calls with a session cookie or a device
//! token, and the sync protocol (check, upload, commit) built on `cloudit-core`.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cloudit_core::chunk::chunker::chunk_slice;
use cloudit_core::chunk::codec::encode_chunk;
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, Manifest, ManifestEntry, Timestamp};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// One client: the server, and the credentials it sends.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    cookie: Option<String>,
    bearer: Option<String>,
}

pub struct Answer {
    pub status: StatusCode,
    pub body: Value,
    pub cookie: Option<String>,
}

impl Api {
    pub fn new(base: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("building the HTTP client")?,
            base: base.trim_end_matches('/').to_owned(),
            cookie: None,
            bearer: None,
        })
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .request(method, format!("{}/api/v1{path}", self.base))
            // The CSRF check wants the server's own origin on unsafe session requests.
            .header(ORIGIN, &self.base);
        if let Some(cookie) = &self.cookie {
            request = request.header(COOKIE, cookie);
        }
        if let Some(token) = &self.bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        request
    }

    async fn answer(response: reqwest::Response) -> Result<Answer> {
        let status = response.status();
        let cookie = response
            .headers()
            .get(SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_owned);
        let bytes = response.bytes().await.context("reading the response")?;
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok(Answer {
            status,
            body,
            cookie,
        })
    }

    /// A JSON call that must succeed (2xx).
    pub async fn json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Answer> {
        let mut request = self.request(method.clone(), path);
        if let Some(body) = body {
            request = request.json_body(&body)?;
        }
        let answer = Self::answer(
            request
                .send()
                .await
                .with_context(|| format!("{method} {path}"))?,
        )
        .await?;
        if !answer.status.is_success() {
            bail!("{method} {path}: {} {}", answer.status, answer.body);
        }
        Ok(answer)
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        Ok(self.json(Method::GET, path, None).await?.body)
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        Ok(self.json(Method::POST, path, Some(body)).await?.body)
    }

    pub async fn patch(&self, path: &str, body: Value) -> Result<Value> {
        Ok(self.json(Method::PATCH, path, Some(body)).await?.body)
    }

    /// Signs in; the session cookie is kept for the next calls.
    pub async fn login(&mut self, email: &str, password: &str) -> Result<()> {
        let answer = self
            .json(
                Method::POST,
                "/auth/login",
                Some(json!({ "email": email, "password": password })),
            )
            .await?;
        self.cookie = answer.cookie;
        Ok(())
    }

    /// Runs the desktop sign-in (loopback + PKCE) without a browser and returns a
    /// client that uses the device token.
    pub async fn device(&self, name: &str, os: &str) -> Result<Self> {
        let verifier = URL_SAFE_NO_PAD.encode(Sha256::digest(name.as_bytes()));
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect = "http://127.0.0.1:53682/callback";
        let authorize = self
            .post(
                "/devices/authorize",
                json!({
                    "code_challenge": challenge,
                    "code_challenge_method": "S256",
                    "redirect_uri": redirect,
                    "state": "seed",
                    "device_name": name,
                    "os": os,
                    "token_ttl": "30d",
                }),
            )
            .await?;
        let url = authorize["redirect_url"]
            .as_str()
            .context("no redirect_url")?;
        let code = url::Url::parse(url)
            .context("redirect_url")?
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .context("no code in the redirect")?;
        let token = self
            .post(
                "/devices/token",
                json!({ "code": code, "code_verifier": verifier, "redirect_uri": redirect }),
            )
            .await?;
        let mut device = self.clone();
        device.cookie = None;
        device.bearer = Some(token["token"].as_str().context("no token")?.to_owned());
        Ok(device)
    }

    /// Chunks `files`, uploads what the folder misses and commits them.
    pub async fn commit(
        &self,
        folder: &str,
        files: &BTreeMap<String, Vec<u8>>,
        parent: Option<&str>,
        kind: &str,
    ) -> Result<String> {
        let (manifest, chunks) = build_manifest(files)?;
        self.upload(folder, &chunks).await?;
        let stored = manifest
            .encode()
            .context("encoding the manifest")?
            .compressed;
        let path = format!(
            "/folders/{folder}/snapshots?parent={}&kind={kind}",
            parent.unwrap_or("none")
        );
        let response = self
            .request(Method::POST, &path)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(stored)
            .send()
            .await
            .context("committing")?;
        let answer = Self::answer(response).await?;
        if !answer.status.is_success() {
            bail!("commit: {} {}", answer.status, answer.body);
        }
        answer.body["snapshot"]["id"]
            .as_str()
            .map(str::to_owned)
            .context("no snapshot id")
    }

    async fn upload(&self, folder: &str, chunks: &BTreeMap<ChunkHash, Vec<u8>>) -> Result<()> {
        let list: Vec<Value> = chunks
            .iter()
            .map(|(hash, data)| json!({ "hash": hash.to_string(), "size": data.len() }))
            .collect();
        let check = self
            .post(
                &format!("/folders/{folder}/chunks/check"),
                json!({ "chunks": list }),
            )
            .await?;
        for item in check["missing"].as_array().context("missing list")? {
            let hash: ChunkHash = item["hash"]
                .as_str()
                .context("hash")?
                .parse()
                .context("hash")?;
            let data = chunks.get(&hash).context("unknown chunk")?;
            let encoded = encode_chunk(data).context("encoding a chunk")?;
            let url = item["upload"]["url"].as_str().context("upload url")?;
            if item["upload"]["kind"] != "proxy" {
                bail!("the seed needs proxy transfers (S3_PRESIGN=false)");
            }
            let at = url.find("/api/v1/").context("an API url")?;
            let response = self
                .request(Method::PUT, &url[at + "/api/v1".len()..])
                .body(encoded)
                .send()
                .await
                .context("uploading a chunk")?;
            if response.status() != StatusCode::NO_CONTENT {
                bail!("chunk upload: {}", response.status());
            }
        }
        Ok(())
    }
}

trait JsonBody: Sized {
    fn json_body(self, body: &Value) -> Result<Self>;
}

impl JsonBody for reqwest::RequestBuilder {
    fn json_body(self, body: &Value) -> Result<Self> {
        Ok(self
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).context("encoding the body")?))
    }
}

fn build_manifest(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(Manifest, BTreeMap<ChunkHash, Vec<u8>>)> {
    let mut chunks = BTreeMap::new();
    let mut entries = Vec::new();
    for (path, data) in files {
        let mut refs = Vec::new();
        for chunk in chunk_slice(data) {
            chunks.insert(chunk.hash, chunk.data.to_vec());
            refs.push(ChunkRef {
                hash: chunk.hash,
                size: u32::try_from(chunk.data.len()).context("chunk size")?,
            });
        }
        let mtime = Timestamp {
            secs: chrono::Utc::now().timestamp(),
            nanos: 0,
        };
        entries.push(ManifestEntry::file(path, mtime, false, refs));
    }
    Ok((
        Manifest::new(entries).context("building the manifest")?,
        chunks,
    ))
}
