//! Committing the scanned local state: as the new head (`normal`, a push) or as a
//! `conflict` snapshot that keeps the local state on the server without moving the head.
//!
//! The uploaded manifest is the local one plus the parent's entries under ignored paths
//! ([`super::plan::upload_manifest`]). Its chunks are uploaded first; the commit is then
//! retried (twice at most) after `chunk_missing` / `chunk_invalid` by uploading the
//! listed chunks again. A `conflict` answer whose current head has exactly this
//! manifest (a retried request whose first attempt landed, or another device that made
//! the same change) counts as success.

use bytes::Bytes;
use cloudit_core::manifest::Manifest;
use cloudit_core::protocol::{ErrorCode, SnapshotInfo, SnapshotKind};

use super::plan::upload_manifest;
use super::upload::{UploadStats, Uploader};
use super::{FolderCtx, Snap};
use crate::api::CommitResult;
use crate::error::{Error, Result};
use crate::progress::Phase;
use crate::scan::Scan;

/// Commit attempts after the first one that answer `chunk_missing` / `chunk_invalid`.
const CHUNK_RETRIES: u32 = 2;

/// A successful commit.
#[derive(Debug, Clone)]
pub struct Committed {
    pub snapshot: SnapshotInfo,
    /// False when the server already had this manifest as its head.
    pub created: bool,
    /// The committed manifest (decoded and stored).
    pub snap: Snap,
    pub uploaded: UploadStats,
}

/// Uploads and commits `scan`'s content on top of `parent_id`.
///
/// `carry_from` supplies the entries under ignored paths (normally the head).
pub async fn commit(
    f: &FolderCtx<'_>,
    scan: &Scan,
    parent_id: Option<&str>,
    carry_from: Option<&Manifest>,
    kind: SnapshotKind,
) -> Result<Committed> {
    scan.require_complete()?;
    let manifest = upload_manifest(&scan.manifest, carry_from, &f.rules)
        .map_err(|e| Error::Protocol(format!("building the manifest: {e}")))?;
    let to_encode = manifest.clone();
    let encoded = tokio::task::spawn_blocking(move || to_encode.encode())
        .await
        .map_err(|e| Error::Protocol(format!("encoding the manifest: {e}")))?
        .map_err(|e| Error::Protocol(format!("encoding the manifest: {e}")))?;

    let uploader = Uploader::new(
        f.api,
        &f.link.folder_id,
        f.link.id,
        &f.link.local_path,
        &scan.manifest,
        f.ctx,
    );
    let mut uploaded = uploader.upload_missing(&manifest).await?;
    let mut retries = 0;
    loop {
        f.ctx.check()?;
        f.ctx.report(f.link.id, Phase::Committing);
        let result = f
            .api
            .commit(
                &f.link.folder_id,
                parent_id,
                kind,
                Bytes::from(encoded.compressed.clone()),
                f.ctx,
            )
            .await;
        let (snapshot, created) = match result {
            Ok(CommitResult::Created(s)) => (s, true),
            Ok(CommitResult::Unchanged(s)) => (s, false),
            Err(error) => {
                let code = error.api_code();
                if matches!(
                    code,
                    Some(ErrorCode::ChunkMissing | ErrorCode::ChunkInvalid)
                ) && retries < CHUNK_RETRIES
                {
                    retries += 1;
                    let hashes = error
                        .api_body()
                        .and_then(|b| b.hashes.clone())
                        .unwrap_or_default();
                    tracing::info!(
                        count = hashes.len(),
                        "the server asks for chunks again, re-uploading"
                    );
                    let again = uploader.upload_hashes(&hashes).await?;
                    uploaded.chunks += again.chunks;
                    uploaded.bytes += again.bytes;
                    continue;
                }
                if code == Some(ErrorCode::Conflict)
                    && let Some(head) = error.api_body().and_then(|b| b.head_snapshot_id.clone())
                    && let Some(head) = f.api.snapshot(&head, f.ctx).await?
                    && head.manifest_hash == encoded.hash
                {
                    (head, false)
                } else {
                    return Err(error);
                }
            }
        };
        return Ok(Committed {
            snapshot: snapshot.clone(),
            created,
            snap: Snap {
                id: snapshot.id,
                manifest,
                stored: encoded.compressed,
            },
            uploaded,
        });
    }
}
