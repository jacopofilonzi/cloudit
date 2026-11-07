//! Chunk transfer (`/api/v1/folders/{id}/chunks*`): which chunks the server lacks, and
//! where to upload or download them.
//!
//! Chunks are stored per scope under their [`ChunkHash`] in the stored chunk encoding
//! (`chunk::codec`: header byte + raw or zstd). A [`Transfer`] says how to move one:
//!
//! - [`TransferKind::Presigned`] (server with `S3_PRESIGN=true`): an S3 URL signed for
//!   a limited time. Send **no** cloudit credentials with it, and only the listed
//!   headers.
//! - [`TransferKind::Proxy`]: a cloudit API URL; send the usual credentials (device
//!   token or session) plus the listed headers.
//!
//! The `url` is absolute when the server knows its public URL, else a path starting with
//! `/api/v1/` that the client resolves against the server URL it uses: clients must
//! handle both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::chunk::MAX_CHUNK_SIZE;
use crate::hash::ChunkHash;

/// Most entries in one [`ChunkCheckRequest`] or [`ChunkDownloadRequest`].
pub const MAX_CHUNKS_PER_REQUEST: usize = 10_000;

/// Largest stored (encoded) chunk accepted: the maximum chunk size, the header byte and
/// room for zstd's worst-case expansion (encoders never store a zstd frame larger than
/// the raw bytes, but decoders accept any frame that decodes within the bounds).
pub const MAX_STORED_CHUNK_BYTES: usize = MAX_CHUNK_SIZE as usize + 1 + 64 * 1024;

/// One chunk the client wants to reference: its hash and uncompressed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkSpec {
    pub hash: ChunkHash,
    /// Uncompressed size, 1 to 1 MiB.
    pub size: u32,
}

/// `POST /api/v1/folders/{id}/chunks/check` (editor or more): at most
/// [`MAX_CHUNKS_PER_REQUEST`] entries (duplicates allowed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkCheckRequest {
    pub chunks: Vec<ChunkSpec>,
}

/// How a [`Transfer`] reaches the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum TransferKind {
    /// Through the cloudit server: send the usual credentials.
    Proxy,
    /// Directly with S3 through a time-limited signed URL: send no credentials.
    Presigned,
}

/// An HTTP request that moves one chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct Transfer {
    pub kind: TransferKind,
    /// `PUT` (upload, body = the stored chunk bytes) or `GET` (download, body = the
    /// stored chunk bytes: decode and verify them with the expected hash).
    pub method: String,
    /// Absolute URL, or a path starting with `/api/v1/` (see the module docs).
    pub url: String,
    /// Headers to send (lowercase names).
    pub headers: BTreeMap<String, String>,
}

/// A chunk the scope lacks, with where to upload it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct MissingChunk {
    pub hash: ChunkHash,
    pub upload: Transfer,
}

/// Answer of `POST /api/v1/folders/{id}/chunks/check`: the chunks to upload before
/// committing (sorted by hash, without duplicates). Uploaded chunks are counted against
/// the scope owner's quota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkCheckResponse {
    pub missing: Vec<MissingChunk>,
}

/// `POST /api/v1/folders/{id}/chunks/download` (viewer or more): at most
/// [`MAX_CHUNKS_PER_REQUEST`] hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkDownloadRequest {
    pub hashes: Vec<ChunkHash>,
}

/// Where to download one chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkDownload {
    pub hash: ChunkHash,
    pub download: Transfer,
}

/// Answer of `POST /api/v1/folders/{id}/chunks/download` (both lists sorted by hash,
/// without duplicates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkDownloadResponse {
    pub chunks: Vec<ChunkDownload>,
    /// Requested hashes the folder's scope doesn't have.
    pub missing: Vec<ChunkHash>,
}
