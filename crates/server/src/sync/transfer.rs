//! Where clients move chunk bytes: a presigned S3 URL when `S3_PRESIGN` is on, else
//! the proxy endpoint `/api/v1/folders/{folder_id}/chunks/{hash}` of this server.
//!
//! Proxy URLs are absolute when the effective public URL is known (`PUBLIC_URL` or the
//! `public_url` setting), else paths the client resolves against the server URL it
//! uses (the server can't know it reliably behind proxies).

use std::collections::BTreeMap;

use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::{Transfer, TransferKind};
use uuid::Uuid;

use super::keys;
use crate::state::AppState;

/// Builds [`Transfer`]s for the chunks of one folder.
pub struct Transfers<'a> {
    state: &'a AppState,
    scope: Uuid,
    /// `<public url>/api/v1/folders/<id>/chunks/` or the same as a path.
    proxy_prefix: String,
}

impl<'a> Transfers<'a> {
    pub async fn new(state: &'a AppState, scope: Uuid, folder: Uuid) -> anyhow::Result<Self> {
        let base = state.public_url().await?.unwrap_or_default();
        Ok(Self {
            state,
            scope,
            proxy_prefix: format!("{base}/api/v1/folders/{folder}/chunks/"),
        })
    }

    fn proxy(&self, method: &str, hash: &ChunkHash) -> Transfer {
        let mut headers = BTreeMap::new();
        if method == "PUT" {
            headers.insert(
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            );
        }
        Transfer {
            kind: TransferKind::Proxy,
            method: method.to_owned(),
            url: format!("{}{hash}", self.proxy_prefix),
            headers,
        }
    }

    fn presigned(method: &str, url: &url::Url) -> Transfer {
        Transfer {
            kind: TransferKind::Presigned,
            method: method.to_owned(),
            url: url.to_string(),
            headers: BTreeMap::new(),
        }
    }

    /// Where to upload `hash`.
    pub async fn upload(&self, hash: &ChunkHash) -> anyhow::Result<Transfer> {
        let key = keys::chunk(self.scope, hash);
        Ok(match self.state.blobs.presign_put(&key).await? {
            Some(url) => Self::presigned("PUT", &url),
            None => self.proxy("PUT", hash),
        })
    }

    /// Where to download `hash`.
    pub async fn download(&self, hash: &ChunkHash) -> anyhow::Result<Transfer> {
        let key = keys::chunk(self.scope, hash);
        Ok(match self.state.blobs.presign_get(&key).await? {
            Some(url) => Self::presigned("GET", &url),
            None => self.proxy("GET", hash),
        })
    }
}
