//! The sync engine proper: scanning a linked folder, comparing it with its base and the
//! server head ([`plan`], [`status`]), uploading ([`upload`], [`push`]) and applying a
//! head safely ([`pull`], [`stage`]).
//!
//! The orchestration (which of these to run, the conflict flow, locks, activity) lives
//! in [`crate::engine`].

pub mod plan;
pub mod pull;
pub mod push;
pub mod stage;
pub mod status;
pub mod upload;

use std::path::Path;

use cloudit_core::manifest::codec::DEFAULT_MAX_MANIFEST_BYTES;
use cloudit_core::manifest::{Manifest, decode_manifest};
use cloudit_core::protocol::FolderInfo;

use crate::api::Api;
use crate::error::{Error, Result};
use crate::progress::{OpContext, Phase};
use crate::scan::{IgnoreRules, Scan, ScanInput, scan};
use crate::state::Db;
use crate::state::folders::LinkedFolder;
use crate::state::index::IndexEntry;

/// A snapshot with its manifest (decoded and stored forms).
#[derive(Debug, Clone)]
pub struct Snap {
    pub id: String,
    pub manifest: Manifest,
    pub stored: Vec<u8>,
}

impl Snap {
    /// Decodes a stored manifest kept in the local state.
    pub fn decode(id: String, stored: Vec<u8>) -> Result<Self> {
        let (manifest, _) = decode_manifest(&stored, DEFAULT_MAX_MANIFEST_BYTES)
            .map_err(|e| Error::State(format!("stored manifest of {id}: {e}")))?;
        Ok(Self {
            id,
            manifest,
            stored,
        })
    }
}

/// Everything an operation on one linked folder needs.
pub struct FolderCtx<'a> {
    pub api: &'a Api,
    pub db: &'a Db,
    pub link: &'a LinkedFolder,
    pub rules: IgnoreRules,
    pub ctx: &'a OpContext,
    /// The engine's chunk cache (downloads).
    pub cache_dir: &'a Path,
}

impl FolderCtx<'_> {
    /// The base snapshot from the local state.
    pub fn base(&self) -> Result<Option<Snap>> {
        self.db
            .base(self.link.id)?
            .map(|(id, stored)| Snap::decode(id, stored))
            .transpose()
    }

    /// This device's pending conflict snapshot.
    pub fn pending_conflict(&self) -> Result<Option<Snap>> {
        self.db
            .pending_conflict(self.link.id)?
            .map(|(id, stored)| Snap::decode(id, stored))
            .transpose()
    }

    /// Scans the folder (on a blocking thread) and refreshes the file index.
    pub async fn scan(&self, base: Option<&Manifest>) -> Result<Scan> {
        let root = self.link.local_path.clone();
        let rules = self.rules.clone();
        let index = self.db.load_index(self.link.id)?;
        let base = base.cloned();
        let cancel = self.ctx.cancel_token().clone();
        let ctx = self.ctx.clone();
        let folder = self.link.id;
        let scanned = tokio::task::spawn_blocking(move || {
            let progress = move |files: u64, hashed_bytes: u64| {
                ctx.report(
                    folder,
                    Phase::Scanning {
                        files,
                        hashed_bytes,
                    },
                );
            };
            scan(&ScanInput {
                root: &root,
                rules: &rules,
                index: &index,
                base: base.as_ref(),
                cancel: &cancel,
                progress: &progress,
            })
        })
        .await
        .map_err(|e| Error::State(format!("scan task: {e}")))??;
        self.db.replace_index(
            self.link.id,
            scanned.index.iter().map(|(p, e)| (p.as_str(), e)),
        )?;
        Ok(scanned)
    }

    /// The server folder and its head (manifest from the base, the cache or the server).
    pub async fn remote(&self, base: Option<&Snap>) -> Result<(FolderInfo, Option<Snap>)> {
        let info = self.api.folder(&self.link.folder_id, self.ctx).await?;
        let Some(head_id) = info.head_snapshot_id.clone() else {
            return Ok((info, None));
        };
        if let Some(base) = base.filter(|b| b.id == head_id) {
            return Ok((info, Some(base.clone())));
        }
        if let Some((cached_id, stored)) = self.db.head_cache(self.link.id)?
            && cached_id == head_id
            && let Ok(snap) = Snap::decode(cached_id, stored)
        {
            return Ok((info, Some(snap)));
        }
        let fetched = self.api.manifest(&head_id, None, self.ctx).await?;
        self.db
            .set_head_cache(self.link.id, &head_id, &fetched.stored)?;
        Ok((
            info,
            Some(Snap {
                id: head_id,
                manifest: fetched.manifest,
                stored: fetched.stored,
            }),
        ))
    }

    /// Records files written or removed by an apply in the index.
    pub fn update_index(&self, written: &[(String, IndexEntry)], removed: &[String]) -> Result<()> {
        self.db.update_index(
            self.link.id,
            written.iter().map(|(p, e)| (p.as_str(), e)),
            removed.iter().map(String::as_str),
        )
    }
}
