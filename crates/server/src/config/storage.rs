//! Blob storage configuration (`STORAGE`, `STORAGE_FS_DIR`, `S3_*`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use url::Url;

use super::env::{Env, invalid};
use super::{ConfigError, Secret};

/// Longest validity SigV4 allows for a presigned URL.
const MAX_PRESIGN_TTL: Duration = Duration::from_secs(7 * 86_400);

/// Where blobs (chunks, manifests) live.
#[derive(Debug, Clone)]
pub enum StorageConfig {
    /// `STORAGE=fs`: a local directory.
    Fs { dir: PathBuf },
    /// `STORAGE=s3`: an S3-compatible bucket (AWS, MinIO, Garage…).
    S3(Box<S3Config>),
}

#[derive(Debug, Clone)]
pub struct S3Config {
    /// `S3_ENDPOINT`: API endpoint as seen by the server; unset means AWS.
    pub endpoint: Option<Url>,
    /// `S3_REGION`, default `us-east-1`.
    pub region: String,
    /// `S3_BUCKET` (required).
    pub bucket: String,
    /// `S3_ACCESS_KEY_ID`; with the secret below. Both unset: credentials come from the
    /// AWS environment (instance metadata, web identity…).
    pub access_key_id: Option<String>,
    /// `S3_SECRET_ACCESS_KEY`.
    pub secret_access_key: Option<Secret>,
    /// `S3_FORCE_PATH_STYLE`, default `true` (`endpoint/bucket/key`; MinIO and Garage need it).
    pub force_path_style: bool,
    /// `S3_PRESIGN`, default `false`: hand clients presigned URLs instead of proxying.
    pub presign: bool,
    /// `S3_PUBLIC_ENDPOINT`: endpoint as reachable by clients; presigned URLs are signed
    /// for this host. Unset: the server endpoint is used.
    pub public_endpoint: Option<Url>,
    /// `S3_PRESIGN_TTL`, default 15 minutes, at most 7 days.
    pub presign_ttl: Duration,
}

fn parse_endpoint(var: &'static str, value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|e| invalid(var, value, e))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(invalid(
            var,
            value,
            "expected an http(s):// URL with a host",
        ));
    }
    Ok(url)
}

impl StorageConfig {
    pub(super) fn from_env<F: Fn(&str) -> Option<String>>(
        env: &Env<F>,
        data_dir: &Path,
    ) -> Result<Self, ConfigError> {
        let kind = env.get("STORAGE").unwrap_or_else(|| "fs".to_owned());
        match kind.as_str() {
            "fs" => {
                if env.bool_or("S3_PRESIGN", false)? {
                    return Err(invalid("S3_PRESIGN", "true", "requires STORAGE=s3"));
                }
                let dir = env
                    .get("STORAGE_FS_DIR")
                    .map_or_else(|| data_dir.join("blobs"), PathBuf::from);
                Ok(Self::Fs { dir })
            }
            "s3" => S3Config::from_env(env).map(|c| Self::S3(Box::new(c))),
            other => Err(invalid("STORAGE", other, "expected `fs` or `s3`")),
        }
    }
}

impl S3Config {
    fn from_env<F: Fn(&str) -> Option<String>>(env: &Env<F>) -> Result<Self, ConfigError> {
        let endpoint = env
            .get("S3_ENDPOINT")
            .map(|v| parse_endpoint("S3_ENDPOINT", &v))
            .transpose()?;
        let public_endpoint = env
            .get("S3_PUBLIC_ENDPOINT")
            .map(|v| parse_endpoint("S3_PUBLIC_ENDPOINT", &v))
            .transpose()?;
        let bucket = env
            .get("S3_BUCKET")
            .ok_or_else(|| invalid("S3_BUCKET", "", "required when STORAGE=s3"))?;
        let access_key_id = env.get("S3_ACCESS_KEY_ID");
        let secret_access_key = env.get("S3_SECRET_ACCESS_KEY").map(Secret::new);
        if access_key_id.is_some() != secret_access_key.is_some() {
            return Err(invalid(
                "S3_ACCESS_KEY_ID",
                "",
                "S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY must be set together",
            ));
        }
        let presign_ttl = env.duration_or("S3_PRESIGN_TTL", Duration::from_secs(15 * 60))?;
        if presign_ttl.is_zero() || presign_ttl > MAX_PRESIGN_TTL {
            return Err(invalid(
                "S3_PRESIGN_TTL",
                &format!("{}s", presign_ttl.as_secs()),
                "must be between 1s and 7d",
            ));
        }
        Ok(Self {
            endpoint,
            region: env
                .get("S3_REGION")
                .unwrap_or_else(|| "us-east-1".to_owned()),
            bucket,
            access_key_id,
            secret_access_key,
            force_path_style: env.bool_or("S3_FORCE_PATH_STYLE", true)?,
            presign: env.bool_or("S3_PRESIGN", false)?,
            public_endpoint,
            presign_ttl,
        })
    }
}
