//! Applying a server snapshot to the local folder (a pull).
//!
//! The caller decides that a pull is allowed: every local file the plan overwrites or
//! deletes must have its content in the [`SafeSet`] (stored on the server); this module
//! refuses otherwise (defense in depth). Then: download the chunks no local file has
//! into the engine's chunk cache (`<data dir>/chunk-cache/<hash>`, verified; small
//! chunks in packs when the server takes them, see [`super::pack`]), stage
//! every file next to the targets, apply with the re-checks of [`super::stage`], record
//! what changed in the file index and, only when everything applied, move the base to
//! the head.
//!
//! Resumable: an interrupted pull (cancellation, a file that changed, a crash) leaves
//! every file either old or new and the base where it was. The next comparison sees the
//! replaced files as converged and the rest as remote changes, so syncing again simply
//! finishes the pull; chunks already in the cache are not downloaded again.

use std::collections::BTreeMap;
use std::path::Path;

use cloudit_core::hash::ChunkHash;
use cloudit_core::protocol::{ChunkDownload, MAX_CHUNKS_PER_REQUEST};
use futures::StreamExt as _;

use super::pack;
use super::plan::{PullPlan, SafeSet, needed_chunks, plan_pull, unsafe_paths};
use super::stage::{ChunkSources, apply, stage};
use super::{FolderCtx, Snap};
use crate::error::{Error, Result};
use crate::progress::Phase;
use crate::scan::Scan;

/// Download requests (single chunks or packs) at the same time.
pub const DOWNLOAD_CONCURRENCY: usize = 6;

/// What a pull changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pulled {
    /// Files and directories written or removed.
    pub changes: u64,
    /// Chunk bytes downloaded (uncompressed).
    pub downloaded_bytes: u64,
}

/// Makes the local folder equal `head` (see the module docs).
pub async fn apply_head(
    f: &FolderCtx<'_>,
    scan: &Scan,
    head: &Snap,
    safe: &SafeSet,
) -> Result<Pulled> {
    scan.require_complete()?;
    let plan = plan_pull(&scan.manifest, &scan.files, &head.manifest, &f.rules);
    let unsafe_paths = unsafe_paths(&plan, safe);
    if !unsafe_paths.is_empty() {
        // The engine stores local changes on the server before pulling: reaching this
        // is a bug, and refusing is the safe outcome.
        return Err(Error::State(format!(
            "refusing to overwrite local changes that are not on the server: {unsafe_paths:?}"
        )));
    }
    let mut pulled = Pulled::default();
    if !plan.is_empty() {
        let sources = ChunkSources::new(&f.link.local_path, f.cache_dir, &scan.manifest);
        let needed = needed_chunks(&plan);
        let to_download: BTreeMap<ChunkHash, u32> = needed
            .iter()
            .filter(|(hash, _)| !sources.has_local(hash) && !sources.cache_path(hash).exists())
            .map(|(h, s)| (*h, *s))
            .collect();
        pulled.downloaded_bytes = download(f, &to_download).await?;
        pulled.changes = plan.len() as u64;
        apply_plan(f, plan, sources).await?;
        remove_cached(f.cache_dir, needed.keys());
    }
    f.db.set_base(f.link.id, &head.id, &head.stored)?;
    Ok(pulled)
}

/// Stages and applies `plan` on a blocking thread; records the changes in the index.
async fn apply_plan(f: &FolderCtx<'_>, plan: PullPlan, sources: ChunkSources) -> Result<()> {
    let root = f.link.local_path.clone();
    let ctx = f.ctx.clone();
    let folder = f.link.id;
    let (applied, result) = tokio::task::spawn_blocking(move || {
        let (temps, staged) = match stage(&root, &plan, &sources, &ctx, folder) {
            Ok(staged) => staged,
            Err(e) => return (super::stage::Applied::default(), Err(e)),
        };
        apply(&root, &plan, temps, &staged, &ctx, folder)
    })
    .await
    .map_err(|e| Error::State(format!("apply task: {e}")))?;
    f.update_index(&applied.written, &applied.removed)?;
    result
}

/// Downloads `chunks` into the cache; the bytes downloaded.
async fn download(f: &FolderCtx<'_>, chunks: &BTreeMap<ChunkHash, u32>) -> Result<u64> {
    if chunks.is_empty() {
        return Ok(0);
    }
    tokio::fs::create_dir_all(f.cache_dir)
        .await
        .map_err(|e| Error::io(format!("creating {}", f.cache_dir.display()), e))?;
    let total_bytes: u64 = chunks.values().map(|s| u64::from(*s)).sum();
    let total_chunks = chunks.len() as u64;
    let (mut done_bytes, mut done_chunks) = (0u64, 0u64);
    let report = |done_bytes, done_chunks| {
        f.ctx.report(
            f.link.id,
            Phase::Downloading {
                done_bytes,
                total_bytes,
                done_chunks,
                total_chunks,
            },
        );
    };

    // Small chunks in packs first (servers at API level 2); what a pack left out and
    // the large chunks go one by one below.
    let items: Vec<(ChunkHash, u32)> = chunks.iter().map(|(h, s)| (*h, *s)).collect();
    let (packs, mut singles) = if f.api.supports_packs() {
        pack::group(items, |(_, size)| *size)
    } else {
        (Vec::new(), items)
    };
    let mut packed = futures::stream::iter(packs.into_iter().map(|group| async move {
        let hashes: Vec<ChunkHash> = group.iter().map(|(h, _)| *h).collect();
        let got = f
            .api
            .download_pack(&f.link.folder_id, &hashes, f.ctx)
            .await?;
        let mut bytes = 0u64;
        for (hash, data) in &got {
            write_cached(f.cache_dir, hash, data).await?;
            bytes += data.len() as u64;
        }
        let left: Vec<(ChunkHash, u32)> = group
            .into_iter()
            .filter(|(h, _)| !got.contains_key(h))
            .collect();
        Ok::<_, Error>((got.len() as u64, bytes, left))
    }))
    .buffer_unordered(DOWNLOAD_CONCURRENCY);
    while let Some(result) = packed.next().await {
        let (count, bytes, left) = result?;
        done_chunks += count;
        done_bytes += bytes;
        singles.extend(left);
        report(done_bytes, done_chunks);
    }
    drop(packed);

    let hashes: Vec<ChunkHash> = singles.iter().map(|(h, _)| *h).collect();
    let mut transfers: Vec<ChunkDownload> = Vec::with_capacity(hashes.len());
    for batch in hashes.chunks(MAX_CHUNKS_PER_REQUEST) {
        f.ctx.check()?;
        let answer = f
            .api
            .download_chunks(&f.link.folder_id, batch, f.ctx)
            .await?;
        if let Some(missing) = answer.missing.first() {
            return Err(Error::Protocol(format!(
                "the server lacks chunk {missing} of its own snapshot"
            )));
        }
        transfers.extend(answer.chunks);
    }
    let mut downloads = futures::stream::iter(transfers.into_iter().map(|t| async move {
        let data = f.api.get_chunk(&t.download, t.hash, f.ctx).await?;
        write_cached(f.cache_dir, &t.hash, &data).await?;
        Ok::<u64, Error>(data.len() as u64)
    }))
    .buffer_unordered(DOWNLOAD_CONCURRENCY);
    while let Some(result) = downloads.next().await {
        done_bytes += result?;
        done_chunks += 1;
        report(done_bytes, done_chunks);
    }
    Ok(done_bytes)
}

/// Puts a verified chunk in the cache (written as `.part`, then renamed).
async fn write_cached(cache_dir: &Path, hash: &ChunkHash, data: &[u8]) -> Result<()> {
    let final_path = cache_dir.join(hash.to_string());
    let part = cache_dir.join(format!("{hash}.part"));
    let written = async {
        tokio::fs::write(&part, data).await?;
        tokio::fs::rename(&part, &final_path).await
    };
    written
        .await
        .map_err(|e| Error::io(format!("writing {}", final_path.display()), e))
}

/// Drops cache entries once their pull completed (best effort).
fn remove_cached<'a>(cache_dir: &Path, hashes: impl Iterator<Item = &'a ChunkHash>) {
    for hash in hashes {
        let _ignored = std::fs::remove_file(cache_dir.join(hash.to_string()));
    }
}
