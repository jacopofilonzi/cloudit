//! Uploading the chunks of a manifest: `chunks/check` in batches of
//! [`MAX_CHUNKS_PER_REQUEST`], then the missing chunks read back from the local files
//! (verified against the scanned hash: a file that changed since the scan stops the
//! upload with [`Error::LocalChanged`]), encoded with `encode_chunk` and sent through
//! their transfer, [`UPLOAD_CONCURRENCY`] requests at a time. With a server at API
//! level 2 the small chunks go in packs instead ([`super::pack`]: one request per pack,
//! retried whole); the large ones keep their transfer.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use cloudit_core::chunk::codec::encode_chunk;
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::Manifest;
use cloudit_core::pack::PackWriter;
use cloudit_core::protocol::{ChunkSpec, MAX_CHUNKS_PER_REQUEST, MissingChunk};
use futures::StreamExt as _;

use super::pack;
use super::stage::local_path;
use crate::api::Api;
use crate::error::{Error, Result};
use crate::progress::{OpContext, Phase};

/// Upload requests (single chunks or packs) at the same time.
pub const UPLOAD_CONCURRENCY: usize = 6;

/// What an upload sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadStats {
    pub chunks: u64,
    /// Uncompressed bytes.
    pub bytes: u64,
}

/// Uploads chunks of one folder from its local files.
pub struct Uploader<'a> {
    pub api: &'a Api,
    pub folder_id: &'a str,
    pub folder: i64,
    pub root: &'a Path,
    pub ctx: &'a OpContext,
    /// Chunk → (path, offset, size) in the scanned local files.
    local: HashMap<ChunkHash, (String, u64, u32)>,
}

impl<'a> Uploader<'a> {
    /// An uploader reading from the files of `scanned` under `root`.
    pub fn new(
        api: &'a Api,
        folder_id: &'a str,
        folder: i64,
        root: &'a Path,
        scanned: &Manifest,
        ctx: &'a OpContext,
    ) -> Self {
        let mut local = HashMap::new();
        for entry in &scanned.entries {
            let mut offset = 0u64;
            for chunk in &entry.chunks {
                local
                    .entry(chunk.hash)
                    .or_insert_with(|| (entry.path.clone(), offset, chunk.size));
                offset += u64::from(chunk.size);
            }
        }
        Self {
            api,
            folder_id,
            folder,
            root,
            ctx,
            local,
        }
    }

    /// Uploads every chunk of `manifest` the server lacks.
    pub async fn upload_missing(&self, manifest: &Manifest) -> Result<UploadStats> {
        let specs: BTreeMap<ChunkHash, u32> = manifest
            .entries
            .iter()
            .flat_map(|e| e.chunks.iter())
            .map(|c| (c.hash, c.size))
            .collect();
        self.upload_specs(&specs).await
    }

    /// Checks and uploads exactly these chunks (after `chunk_missing` / `chunk_invalid`).
    pub async fn upload_hashes(&self, hashes: &[ChunkHash]) -> Result<UploadStats> {
        let specs: BTreeMap<ChunkHash, u32> = hashes
            .iter()
            .filter_map(|h| self.local.get(h).map(|(_, _, size)| (*h, *size)))
            .collect();
        if specs.len() < hashes.len() {
            return Err(Error::Protocol(
                "the server lacks a chunk this device doesn't have".to_owned(),
            ));
        }
        self.upload_specs(&specs).await
    }

    async fn upload_specs(&self, specs: &BTreeMap<ChunkHash, u32>) -> Result<UploadStats> {
        let all: Vec<ChunkSpec> = specs
            .iter()
            .map(|(hash, size)| ChunkSpec {
                hash: *hash,
                size: *size,
            })
            .collect();
        let mut missing: Vec<MissingChunk> = Vec::new();
        for batch in all.chunks(MAX_CHUNKS_PER_REQUEST) {
            self.ctx.check()?;
            self.ctx.report(
                self.folder,
                Phase::Checking {
                    chunks: batch.len() as u64,
                },
            );
            let answer = self
                .api
                .check_chunks(self.folder_id, batch, self.ctx)
                .await?;
            missing.extend(answer.missing);
        }
        let total_bytes: u64 = missing
            .iter()
            .map(|m| u64::from(specs.get(&m.hash).copied().unwrap_or(0)))
            .sum();
        let total_chunks = missing.len() as u64;
        let mut stats = UploadStats::default();
        self.ctx.report(
            self.folder,
            Phase::Uploading {
                done_bytes: 0,
                total_bytes,
                done_chunks: 0,
                total_chunks,
            },
        );
        // Small chunks go in packs when the server takes them (whatever their transfer
        // says: a pack goes through the server even with presigned URLs).
        let (packs, singles) = if self.api.supports_packs() {
            pack::group(missing, |m| specs.get(&m.hash).copied().unwrap_or(u32::MAX))
        } else {
            (Vec::new(), missing)
        };
        let jobs = packs
            .into_iter()
            .map(Job::Pack)
            .chain(singles.into_iter().map(Job::One));
        let mut uploads = futures::stream::iter(jobs.map(|job| async move {
            match job {
                Job::Pack(group) => self.upload_pack(group).await,
                Job::One(missing) => Ok((1, u64::from(self.upload_one(missing).await?))),
            }
        }))
        .buffer_unordered(UPLOAD_CONCURRENCY);
        while let Some(result) = uploads.next().await {
            let (chunks, bytes) = result?;
            stats.chunks += chunks;
            stats.bytes += bytes;
            self.ctx.report(
                self.folder,
                Phase::Uploading {
                    done_bytes: stats.bytes,
                    total_bytes,
                    done_chunks: stats.chunks,
                    total_chunks,
                },
            );
        }
        Ok(stats)
    }

    /// Where a missing chunk is in the local files.
    fn source(&self, hash: &ChunkHash) -> Result<(String, u64, u32)> {
        self.local.get(hash).cloned().ok_or_else(|| {
            Error::Protocol(format!(
                "the server lacks chunk {hash}, which no local file has"
            ))
        })
    }

    /// Reads, encodes and uploads small chunks as one pack; the chunks and uncompressed
    /// bytes sent.
    async fn upload_pack(&self, group: Vec<MissingChunk>) -> Result<(u64, u64)> {
        let mut sources = Vec::with_capacity(group.len());
        for missing in &group {
            let (path, offset, size) = self.source(&missing.hash)?;
            sources.push((
                local_path(self.root, &path),
                path,
                offset,
                size,
                missing.hash,
            ));
        }
        let bytes: u64 = sources.iter().map(|s| u64::from(s.3)).sum();
        let count = sources.len() as u64;
        let body = self
            .ctx
            .run(async move {
                tokio::task::spawn_blocking(move || {
                    let mut writer = PackWriter::new();
                    for (full, path, offset, size, hash) in &sources {
                        let stored = read_encoded(full, path, *offset, *size, *hash)?;
                        writer
                            .push(hash, &stored)
                            .map_err(|e| Error::Protocol(format!("building a chunk pack: {e}")))?;
                    }
                    Ok::<_, Error>(writer.finish())
                })
                .await
                .map_err(|e| Error::Protocol(format!("reading chunks: {e}")))?
            })
            .await?;
        let answer = self
            .api
            .upload_pack(self.folder_id, Bytes::from(body), self.ctx)
            .await?;
        if u64::from(answer.chunks) != count {
            return Err(Error::Protocol(format!(
                "the server read {} chunks of a pack of {count}",
                answer.chunks
            )));
        }
        Ok((count, bytes))
    }

    /// Reads, encodes and uploads one chunk; its uncompressed size.
    async fn upload_one(&self, missing: MissingChunk) -> Result<u32> {
        let (path, offset, size) = self.source(&missing.hash)?;
        let full = local_path(self.root, &path);
        let hash = missing.hash;
        let stored = self
            .ctx
            .run(async move {
                tokio::task::spawn_blocking(move || read_encoded(&full, &path, offset, size, hash))
                    .await
                    .map_err(|e| Error::Protocol(format!("reading a chunk: {e}")))?
            })
            .await?;
        self.api
            .put_chunk(&missing.upload, Bytes::from(stored), self.ctx)
            .await?;
        Ok(size)
    }
}

/// One upload request: a pack of small chunks or one chunk through its transfer.
enum Job {
    Pack(Vec<MissingChunk>),
    One(MissingChunk),
}

/// One chunk of a local file, verified and encoded for storage.
fn read_encoded(
    full: &PathBuf,
    path: &str,
    offset: u64,
    size: u32,
    hash: ChunkHash,
) -> Result<Vec<u8>> {
    let changed = || Error::LocalChanged {
        path: path.to_owned(),
    };
    let mut file = File::open(full).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            changed()
        } else {
            Error::ScanProblems(vec![crate::scan::ScanProblem {
                path: path.to_owned(),
                kind: crate::scan::ProblemKind::Unreadable,
                detail: e.to_string(),
            }])
        }
    })?;
    file.seek(SeekFrom::Start(offset)).map_err(|_| changed())?;
    let mut bytes = vec![0u8; size as usize];
    file.read_exact(&mut bytes).map_err(|_| changed())?;
    if ChunkHash::of(&bytes) != hash {
        return Err(changed());
    }
    encode_chunk(&bytes).map_err(|e| Error::Protocol(format!("encoding a chunk: {e}")))
}
