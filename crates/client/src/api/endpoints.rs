//! Typed API calls.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use cloudit_core::chunk::codec::decode_chunk;
use cloudit_core::hash::{ChunkHash, ManifestHash};
use cloudit_core::manifest::codec::DEFAULT_MAX_MANIFEST_BYTES;
use cloudit_core::manifest::{Manifest, decode_manifest};
use cloudit_core::pack::{MAX_PACK_BYTES, PACK_CONTENT_TYPE, decode_pack};
use cloudit_core::protocol::{
    ChunkCheckRequest, ChunkCheckResponse, ChunkDownloadRequest, ChunkDownloadResponse,
    ChunkPackRequest, ChunkPackUploadResponse, ChunkSpec, CommitResponse, CreateFolderRequest,
    CreateScopeRequest, DeviceTokenRequest, DeviceTokenResponse, FolderInfo, HealthResponse,
    MAX_STORED_CHUNK_BYTES, RestoreSnapshotRequest, ScopeInfo, SnapshotInfo, SnapshotKind,
    SyncHeads, Transfer, TransferKind, UserProfile,
};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use reqwest::{Method, Response, StatusCode};

use super::{Api, Call, TRANSFER_TIMEOUT, error_of, read_bounded};
use crate::error::{Error, Result};
use crate::progress::OpContext;

/// Answer of a commit.
#[derive(Debug, Clone)]
pub enum CommitResult {
    /// 201: a new snapshot.
    Created(SnapshotInfo),
    /// 200: the head already has this manifest (nothing created).
    Unchanged(SnapshotInfo),
}

impl CommitResult {
    #[must_use]
    pub fn snapshot(&self) -> &SnapshotInfo {
        match self {
            Self::Created(s) | Self::Unchanged(s) => s,
        }
    }
}

/// A downloaded manifest: decoded, its hash, and the stored bytes as served.
#[derive(Debug, Clone)]
pub struct FetchedManifest {
    pub manifest: Manifest,
    pub hash: ManifestHash,
    pub stored: Vec<u8>,
}

impl Api {
    /// `GET /api/v1/health` (no credentials).
    pub async fn health(&self, ctx: &OpContext) -> Result<HealthResponse> {
        self.call_json(
            &Call::new(Method::GET, self.url("health")?).anonymous(),
            ctx,
        )
        .await
    }

    /// `POST /devices/token`.
    pub async fn device_token(
        &self,
        request: &DeviceTokenRequest,
        ctx: &OpContext,
    ) -> Result<DeviceTokenResponse> {
        let call = Call::new(Method::POST, self.url("devices/token")?)
            .anonymous()
            .json(request)?;
        self.call_json(&call, ctx).await
    }

    /// `GET /me`.
    pub async fn me(&self, ctx: &OpContext) -> Result<UserProfile> {
        self.call_json(&Call::new(Method::GET, self.url("me")?), ctx)
            .await
    }

    /// `GET /scopes`.
    pub async fn scopes(&self, ctx: &OpContext) -> Result<Vec<ScopeInfo>> {
        self.call_json(&Call::new(Method::GET, self.url("scopes")?), ctx)
            .await
    }

    /// `GET /scopes/{id}`.
    pub async fn scope(&self, id: &str, ctx: &OpContext) -> Result<ScopeInfo> {
        self.call_json(
            &Call::new(Method::GET, self.url(&format!("scopes/{id}"))?),
            ctx,
        )
        .await
    }

    /// `POST /scopes`.
    pub async fn create_scope(&self, name: &str, ctx: &OpContext) -> Result<ScopeInfo> {
        let call = Call::new(Method::POST, self.url("scopes")?).json(&CreateScopeRequest {
            name: name.to_owned(),
        })?;
        self.call_json(&call, ctx).await
    }

    /// `GET /scopes/{id}/folders`.
    pub async fn folders(&self, scope: &str, ctx: &OpContext) -> Result<Vec<FolderInfo>> {
        self.call_json(
            &Call::new(Method::GET, self.url(&format!("scopes/{scope}/folders"))?),
            ctx,
        )
        .await
    }

    /// `POST /scopes/{id}/folders`.
    pub async fn create_folder(
        &self,
        scope: &str,
        request: &CreateFolderRequest,
        ctx: &OpContext,
    ) -> Result<FolderInfo> {
        let call =
            Call::new(Method::POST, self.url(&format!("scopes/{scope}/folders"))?).json(request)?;
        self.call_json(&call, ctx).await
    }

    /// `GET /folders/{id}`.
    pub async fn folder(&self, id: &str, ctx: &OpContext) -> Result<FolderInfo> {
        self.call_json(
            &Call::new(Method::GET, self.url(&format!("folders/{id}"))?),
            ctx,
        )
        .await
    }

    /// `GET /snapshots/{id}`; `None` if it doesn't exist (any more).
    pub async fn snapshot(&self, id: &str, ctx: &OpContext) -> Result<Option<SnapshotInfo>> {
        match self
            .call_json(
                &Call::new(Method::GET, self.url(&format!("snapshots/{id}"))?),
                ctx,
            )
            .await
        {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(e) if e.kind() == crate::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `GET /snapshots/{id}/manifest`, decoded and checked against `expected`.
    pub async fn manifest(
        &self,
        snapshot: &str,
        expected: Option<&ManifestHash>,
        ctx: &OpContext,
    ) -> Result<FetchedManifest> {
        let call = Call::new(
            Method::GET,
            self.url(&format!("snapshots/{snapshot}/manifest"))?,
        )
        .timeout(TRANSFER_TIMEOUT);
        let response = self.send(&call, ctx).await?;
        if !response.status().is_success() {
            return Err(error_of(response).await);
        }
        let stored = read_bounded(response, DEFAULT_MAX_MANIFEST_BYTES).await?;
        let (stored, decoded) = tokio::task::spawn_blocking(move || {
            let decoded = decode_manifest(&stored, DEFAULT_MAX_MANIFEST_BYTES);
            (stored, decoded)
        })
        .await
        .map_err(|e| Error::Protocol(format!("decoding a manifest: {e}")))?;
        let (manifest, hash) =
            decoded.map_err(|e| Error::Protocol(format!("manifest of {snapshot}: {e}")))?;
        if expected.is_some_and(|expected| *expected != hash) {
            return Err(Error::Protocol(format!(
                "manifest of {snapshot} doesn't match its hash"
            )));
        }
        Ok(FetchedManifest {
            manifest,
            hash,
            stored,
        })
    }

    /// `POST /folders/{id}/chunks/check`.
    pub async fn check_chunks(
        &self,
        folder: &str,
        chunks: &[ChunkSpec],
        ctx: &OpContext,
    ) -> Result<ChunkCheckResponse> {
        let call = Call::new(
            Method::POST,
            self.url(&format!("folders/{folder}/chunks/check"))?,
        )
        .json(&ChunkCheckRequest {
            chunks: chunks.to_vec(),
        })?;
        self.call_json(&call, ctx).await
    }

    /// `POST /folders/{id}/chunks/download`.
    pub async fn download_chunks(
        &self,
        folder: &str,
        hashes: &[ChunkHash],
        ctx: &OpContext,
    ) -> Result<ChunkDownloadResponse> {
        let call = Call::new(
            Method::POST,
            self.url(&format!("folders/{folder}/chunks/download"))?,
        )
        .json(&ChunkDownloadRequest {
            hashes: hashes.to_vec(),
        })?;
        self.call_json(&call, ctx).await
    }

    /// `POST /folders/{id}/snapshots?parent=…&kind=…` with a stored manifest.
    pub async fn commit(
        &self,
        folder: &str,
        parent: Option<&str>,
        kind: SnapshotKind,
        stored_manifest: Bytes,
        ctx: &OpContext,
    ) -> Result<CommitResult> {
        let mut url = self.url(&format!("folders/{folder}/snapshots"))?;
        url.query_pairs_mut()
            .append_pair("parent", parent.unwrap_or("none"))
            .append_pair("kind", kind.as_str());
        let call = Call::new(Method::POST, url)
            .header("content-type", "application/octet-stream")
            .raw(stored_manifest)
            .timeout(TRANSFER_TIMEOUT);
        let response = self.send(&call, ctx).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(error_of(response).await);
        }
        let bytes = read_bounded(response, super::MAX_JSON_BYTES).await?;
        let answer: CommitResponse = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Protocol(format!("commit answer: {e}")))?;
        Ok(if answer.unchanged {
            CommitResult::Unchanged(answer.snapshot)
        } else {
            CommitResult::Created(answer.snapshot)
        })
    }

    /// `POST /snapshots/{id}/restore`.
    pub async fn restore(
        &self,
        snapshot: &str,
        expected_head: Option<&str>,
        ctx: &OpContext,
    ) -> Result<SnapshotInfo> {
        let call = Call::new(
            Method::POST,
            self.url(&format!("snapshots/{snapshot}/restore"))?,
        )
        .json(&RestoreSnapshotRequest {
            expected_head: expected_head.map(str::to_owned),
        })?;
        self.call_json(&call, ctx).await
    }

    /// `POST /snapshots/{id}/resolve` ("keep the server version").
    pub async fn resolve(&self, snapshot: &str, ctx: &OpContext) -> Result<SnapshotInfo> {
        self.call_json(
            &Call::new(
                Method::POST,
                self.url(&format!("snapshots/{snapshot}/resolve"))?,
            ),
            ctx,
        )
        .await
    }

    /// `GET /sync/heads`; `None` when unchanged since `etag` (304). The new ETag comes
    /// with the answer.
    pub async fn heads(
        &self,
        etag: Option<&str>,
        ctx: &OpContext,
    ) -> Result<Option<(SyncHeads, Option<String>)>> {
        let mut call = Call::new(Method::GET, self.url("sync/heads")?);
        if let Some(etag) = etag {
            call = call.header(IF_NONE_MATCH.as_str(), etag);
        }
        let response = self.send(&call, ctx).await?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(error_of(response).await);
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = read_bounded(response, super::MAX_JSON_BYTES).await?;
        let heads = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Protocol(format!("sync heads: {e}")))?;
        Ok(Some((heads, etag)))
    }

    /// `GET /events`: the open SSE response (no total timeout; the read timeout of the
    /// client notices a dead connection).
    pub async fn events(&self, ctx: &OpContext) -> Result<Response> {
        let call = Call::new(Method::GET, self.url("events")?)
            .header("accept", "text/event-stream")
            .timeout(std::time::Duration::from_secs(365 * 86_400));
        let response = self.send(&call, ctx).await?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(error_of(response).await)
        }
    }

    /// The call for a chunk transfer: credentials only for proxy URLs on the server's
    /// own origin.
    fn transfer_call(&self, transfer: &Transfer, method: Method) -> Result<Call> {
        let url = self.transfer_url(&transfer.url)?;
        let same_origin = url.origin() == self.base.origin();
        let mut call = Call::new(method, url).timeout(TRANSFER_TIMEOUT);
        if transfer.kind == TransferKind::Presigned || !same_origin {
            call = call.anonymous();
        }
        for (name, value) in &transfer.headers {
            call = call.header(name, value);
        }
        Ok(call)
    }

    /// Uploads one stored chunk through its transfer.
    pub async fn put_chunk(
        &self,
        transfer: &Transfer,
        stored: Bytes,
        ctx: &OpContext,
    ) -> Result<()> {
        let call = self.transfer_call(transfer, Method::PUT)?.raw(stored);
        self.call_empty(&call, ctx).await
    }

    /// `POST /folders/{id}/chunks/pack` (API level 2) with a pack body (`PackWriter`).
    pub async fn upload_pack(
        &self,
        folder: &str,
        pack: Bytes,
        ctx: &OpContext,
    ) -> Result<ChunkPackUploadResponse> {
        let call = Call::new(
            Method::POST,
            self.url(&format!("folders/{folder}/chunks/pack"))?,
        )
        .header("content-type", PACK_CONTENT_TYPE)
        .raw(pack)
        .timeout(TRANSFER_TIMEOUT);
        self.call_json(&call, ctx).await
    }

    /// `POST /folders/{id}/chunks/pack/download` (API level 2): the requested chunks the
    /// answer carried, each decoded against its hash (corrupt data, or a record nobody
    /// asked for, is an error). Requested hashes the answer left out are not in the map:
    /// the caller fetches them one by one.
    pub async fn download_pack(
        &self,
        folder: &str,
        hashes: &[ChunkHash],
        ctx: &OpContext,
    ) -> Result<HashMap<ChunkHash, Vec<u8>>> {
        let call = Call::new(
            Method::POST,
            self.url(&format!("folders/{folder}/chunks/pack/download"))?,
        )
        .json(&ChunkPackRequest {
            hashes: hashes.to_vec(),
        })?
        .timeout(TRANSFER_TIMEOUT);
        let response = self.send(&call, ctx).await?;
        if !response.status().is_success() {
            return Err(error_of(response).await);
        }
        let body = read_bounded(response, MAX_PACK_BYTES).await?;
        let wanted: HashSet<ChunkHash> = hashes.iter().copied().collect();
        tokio::task::spawn_blocking(move || {
            let entries =
                decode_pack(&body).map_err(|e| Error::Protocol(format!("chunk pack: {e}")))?;
            let mut out = HashMap::with_capacity(entries.len());
            for entry in entries {
                if !wanted.contains(&entry.hash) {
                    return Err(Error::Protocol(format!(
                        "chunk pack carries {}, which was not requested",
                        entry.hash
                    )));
                }
                let data = decode_chunk(&body[entry.range], &entry.hash)
                    .map_err(|e| Error::Protocol(format!("chunk {}: {e}", entry.hash)))?;
                out.insert(entry.hash, data);
            }
            Ok(out)
        })
        .await
        .map_err(|e| Error::Protocol(format!("decoding a chunk pack: {e}")))?
    }

    /// Downloads one chunk through its transfer and decodes it against `hash`
    /// (corrupt or wrong data is an error, never accepted).
    pub async fn get_chunk(
        &self,
        transfer: &Transfer,
        hash: ChunkHash,
        ctx: &OpContext,
    ) -> Result<Vec<u8>> {
        let call = self.transfer_call(transfer, Method::GET)?;
        let response = self.send(&call, ctx).await?;
        if !response.status().is_success() {
            return Err(error_of(response).await);
        }
        let stored = read_bounded(response, MAX_STORED_CHUNK_BYTES).await?;
        tokio::task::spawn_blocking(move || decode_chunk(&stored, &hash))
            .await
            .map_err(|e| Error::Protocol(format!("decoding a chunk: {e}")))?
            .map_err(|e| Error::Protocol(format!("chunk {hash}: {e}")))
    }
}
