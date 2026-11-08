//! Blob storage: [`BlobStore`] wraps an `object_store` backend chosen by `STORAGE`.
//!
//! - `fs`: `LocalFileSystem` rooted at `STORAGE_FS_DIR` (default `<DATA_DIR>/blobs`,
//!   created at startup). Writes are atomic (temp file + rename) and fsynced, because
//!   the database may reference a blob as soon as `put` returns.
//! - `s3`: any S3-compatible service (AWS, MinIO, Garage…), path-style by default.
//!
//! Keys are `/`-separated object paths (`scopes/{scope_id}/chunks/{sha256}`, layout in `sync`), validated
//! by [`object_store::path::Path::parse`]: no empty, `.` or `..` segments.
//!
//! # Presigned URLs
//!
//! With `S3_PRESIGN=true`, [`BlobStore::presign_put`] / [`BlobStore::presign_get`] return
//! SigV4 query-signed URLs so clients transfer blobs directly with S3; otherwise (and
//! always with `fs`) they return `None` and the server proxies the bytes.
//!
//! SigV4 signs the `Host` header, so a URL must be signed for the host the client will
//! actually contact. When `S3_PUBLIC_ENDPOINT` differs from `S3_ENDPOINT` (e.g. the
//! server reaches MinIO as `http://minio:9000`, clients as `https://s3.example.com`),
//! a second, signer-only `AmazonS3` is built with the public endpoint and the same
//! credentials: signing is a local computation, so that store never makes requests.
//! A reverse proxy in front of S3 must forward the `Host` header unchanged.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::stream::{BoxStream, StreamExt as _, TryStreamExt as _};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use object_store::signer::Signer as _;
use object_store::{ObjectStore, ObjectStoreExt as _, PutPayload};
use url::Url;

use crate::config::{S3Config, StorageConfig};

/// Metadata of one stored blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    pub key: String,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
}

impl From<object_store::ObjectMeta> for BlobMeta {
    fn from(meta: object_store::ObjectMeta) -> Self {
        Self {
            key: meta.location.to_string(),
            size: meta.size,
            last_modified: meta.last_modified,
        }
    }
}

#[derive(Debug, Clone)]
enum Backend {
    Fs { root: PathBuf },
    S3 { bucket: String },
}

#[derive(Debug, Clone)]
struct Presigner {
    signer: Arc<AmazonS3>,
    ttl: Duration,
}

/// Shared handle to blob storage; cheap to clone.
#[derive(Debug, Clone)]
pub struct BlobStore {
    store: Arc<dyn ObjectStore>,
    backend: Backend,
    presigner: Option<Presigner>,
}

fn parse_key(key: &str) -> anyhow::Result<ObjectPath> {
    let path = ObjectPath::parse(key).with_context(|| format!("invalid blob key {key:?}"))?;
    // `parse` tolerates a leading/trailing `/`: require the canonical form, so one blob
    // has exactly one key.
    anyhow::ensure!(
        !key.is_empty() && path.as_ref() == key,
        "invalid blob key {key:?}: not in canonical form"
    );
    Ok(path)
}

fn s3_builder(config: &S3Config, endpoint: Option<&Url>) -> AmazonS3Builder {
    let mut builder = AmazonS3Builder::new()
        .with_region(&config.region)
        .with_bucket_name(&config.bucket)
        .with_virtual_hosted_style_request(!config.force_path_style);
    if let Some(endpoint) = endpoint {
        builder = builder
            .with_endpoint(endpoint.as_str().trim_end_matches('/'))
            .with_allow_http(endpoint.scheme() == "http");
    }
    if let (Some(key_id), Some(secret)) = (&config.access_key_id, &config.secret_access_key) {
        builder = builder
            .with_access_key_id(key_id)
            .with_secret_access_key(secret.expose());
    }
    builder
}

impl BlobStore {
    /// Builds the store from configuration. For `fs` the root directory is created;
    /// for `s3` nothing is contacted (reachability shows up in `/ready`).
    pub fn from_config(config: &StorageConfig) -> anyhow::Result<Self> {
        match config {
            StorageConfig::Fs { dir } => Self::fs(dir.clone()),
            StorageConfig::S3(s3) => Self::s3(s3),
        }
    }

    fn fs(root: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating STORAGE_FS_DIR {}", root.display()))?;
        let store = LocalFileSystem::new_with_prefix(&root)
            .with_context(|| format!("opening STORAGE_FS_DIR {}", root.display()))?
            .with_automatic_cleanup(true)
            .with_fsync(true);
        Ok(Self {
            store: Arc::new(store),
            backend: Backend::Fs { root },
            presigner: None,
        })
    }

    fn s3(config: &S3Config) -> anyhow::Result<Self> {
        let store = s3_builder(config, config.endpoint.as_ref())
            .build()
            .context("configuring S3 storage (S3_* variables)")?;
        let store = Arc::new(store);
        let presigner = if config.presign {
            let signer = match &config.public_endpoint {
                Some(public) => Arc::new(
                    s3_builder(config, Some(public))
                        .build()
                        .context("configuring the S3 presigner (S3_PUBLIC_ENDPOINT)")?,
                ),
                None => Arc::clone(&store),
            };
            Some(Presigner {
                signer,
                ttl: config.presign_ttl,
            })
        } else {
            None
        };
        Ok(Self {
            store,
            backend: Backend::S3 {
                bucket: config.bucket.clone(),
            },
            presigner,
        })
    }

    /// Short description for logs (`fs:<dir>` or `s3:<bucket>`).
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.backend {
            Backend::Fs { root } => format!("fs:{}", root.display()),
            Backend::S3 { bucket } => format!("s3:{bucket}"),
        }
    }

    /// Whether presigned URLs are enabled.
    #[must_use]
    pub fn presign_enabled(&self) -> bool {
        self.presigner.is_some()
    }

    /// Stores `data` at `key` atomically, replacing any existing blob.
    pub async fn put(&self, key: &str, data: Bytes) -> anyhow::Result<()> {
        let path = parse_key(key)?;
        self.store
            .put(&path, PutPayload::from(data))
            .await
            .with_context(|| format!("storing blob {key}"))?;
        Ok(())
    }

    /// Streams the blob at `key`; `None` if it doesn't exist.
    pub async fn get(
        &self,
        key: &str,
    ) -> anyhow::Result<Option<BoxStream<'static, anyhow::Result<Bytes>>>> {
        let path = parse_key(key)?;
        match self.store.get(&path).await {
            Ok(result) => {
                let key = key.to_owned();
                let stream = result
                    .into_stream()
                    .map_err(move |e| anyhow::Error::new(e).context(format!("reading blob {key}")))
                    .boxed();
                Ok(Some(stream))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(anyhow::Error::new(e).context(format!("reading blob {key}"))),
        }
    }

    /// Reads a whole blob into memory; `None` if it doesn't exist.
    pub async fn get_bytes(&self, key: &str) -> anyhow::Result<Option<Bytes>> {
        let Some(mut stream) = self.get(key).await? else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        while let Some(part) = stream.next().await {
            buf.extend_from_slice(&part?);
        }
        Ok(Some(Bytes::from(buf)))
    }

    /// Metadata of the blob at `key`; `None` if it doesn't exist.
    pub async fn head(&self, key: &str) -> anyhow::Result<Option<BlobMeta>> {
        let path = parse_key(key)?;
        match self.store.head(&path).await {
            Ok(meta) => Ok(Some(meta.into())),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(anyhow::Error::new(e).context(format!("reading blob metadata {key}"))),
        }
    }

    /// Deletes the blob at `key`. Deleting a missing blob is not an error.
    pub async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let path = parse_key(key)?;
        match self.store.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(anyhow::Error::new(e).context(format!("deleting blob {key}"))),
        }
    }

    /// Streams every blob under `prefix` (whole path segments: `a/b` matches `a/b/c`,
    /// not `a/bc`), in no particular order.
    pub fn list(
        &self,
        prefix: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<BlobMeta>>> {
        let prefix_path = parse_key(prefix)?;
        let prefix = prefix.to_owned();
        Ok(self
            .store
            .list(Some(&prefix_path))
            .map(move |item| {
                item.map(BlobMeta::from).map_err(|e| {
                    anyhow::Error::new(e).context(format!("listing blobs under {prefix}"))
                })
            })
            .boxed())
    }

    /// A URL the client can `PUT` the blob to directly; `None` if presigning is off.
    pub async fn presign_put(&self, key: &str) -> anyhow::Result<Option<Url>> {
        self.presign(object_store::signer::Method::PUT, key).await
    }

    /// A URL the client can `GET` the blob from directly; `None` if presigning is off.
    pub async fn presign_get(&self, key: &str) -> anyhow::Result<Option<Url>> {
        self.presign(object_store::signer::Method::GET, key).await
    }

    async fn presign(
        &self,
        method: object_store::signer::Method,
        key: &str,
    ) -> anyhow::Result<Option<Url>> {
        let Some(presigner) = &self.presigner else {
            return Ok(None);
        };
        let path = parse_key(key)?;
        let url = presigner
            .signer
            .signed_url(method, &path, presigner.ttl)
            .await
            .with_context(|| format!("presigning blob {key}"))?;
        Ok(Some(url))
    }

    /// Readiness check. `fs`: the root is a directory and a probe file can be written
    /// and removed. `s3`: listing a probe prefix succeeds (bucket exists, credentials
    /// accepted).
    pub async fn check(&self) -> anyhow::Result<()> {
        match &self.backend {
            Backend::Fs { root } => {
                let meta = tokio::fs::metadata(root)
                    .await
                    .with_context(|| format!("reading {}", root.display()))?;
                anyhow::ensure!(meta.is_dir(), "{} is not a directory", root.display());
                let probe = root.join(format!(".ready-probe-{}", uuid::Uuid::new_v4().simple()));
                tokio::fs::write(&probe, b"ok")
                    .await
                    .with_context(|| format!("writing {}", probe.display()))?;
                tokio::fs::remove_file(&probe)
                    .await
                    .with_context(|| format!("removing {}", probe.display()))?;
                Ok(())
            }
            Backend::S3 { bucket } => {
                let prefix = ObjectPath::from("_ready");
                self.store
                    .list_with_delimiter(Some(&prefix))
                    .await
                    .with_context(|| format!("listing bucket {bucket}"))?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
