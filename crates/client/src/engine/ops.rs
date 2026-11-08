//! Status, sync and conflict decisions of a linked folder.
//!
//! [`Engine::sync`] acts on the status (see [`crate::sync::status::SyncState`]):
//!
//! - `local_changes` → push (commit on top of the head the comparison used: every remote
//!   change is already in the local content);
//! - `remote_changes` → pull;
//! - `in_sync` / `server_conflict` → only moves the base to the head if they differ;
//! - `conflict` → uploads the local state as a `conflict` snapshot (parent = base),
//!   unless this device already did for the same content, and answers
//!   [`SyncOutcome::NeedsDecision`]: the UI then calls [`Engine::keep_local`],
//!   [`Engine::keep_server`] or [`Engine::decide_later`]. Conflicts made by other
//!   devices (`server_conflict`) can be decided with the same two calls.
//!
//! **The safety rule** is enforced where files are replaced: before a pull, every local
//! file it would overwrite or delete must have content stored on the server (in the
//! head, in the base if that snapshot still exists, or in this device's pending
//! conflict snapshot). Otherwise the local state is first committed as a `conflict`
//! snapshot; if that fails, nothing is touched. When the only reason was that retention
//! deleted the base snapshot, that safety copy is resolved right after the pull (it
//! stays in the history like any version).

use cloudit_core::protocol::SnapshotKind;
use serde::Serialize;

use super::Engine;
use crate::error::{Error, ErrorKind, Result};
use crate::progress::OpContext;
use crate::scan::{IgnoreRules, Scan};
use crate::state::activity::NewActivity;
use crate::state::folders::LinkedFolder;
use crate::sync::plan::{SafeSet, compare, plan_pull, unsafe_paths};
use crate::sync::status::{FolderStatus, SyncState, state_of};
use crate::sync::{FolderCtx, Snap, pull, push};

/// What [`Engine::sync`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Nothing to do (or nothing possible: see the status, e.g. a missing local path).
    UpToDate,
    /// The local content became the new head.
    Pushed {
        snapshot_id: String,
        uploaded_chunks: u64,
        uploaded_bytes: u64,
    },
    /// The head was applied locally.
    Pulled {
        snapshot_id: String,
        changes: u64,
        downloaded_bytes: u64,
    },
    /// Both sides changed: the local state is stored as this conflict snapshot; the
    /// user decides.
    NeedsDecision { conflict_id: String },
}

/// The outcome of an operation and the folder's status after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub outcome: SyncOutcome,
    pub status: FolderStatus,
}

/// A folder examined: everything the decisions need.
struct Examined {
    base: Option<Snap>,
    pending: Option<Snap>,
    head: Option<Snap>,
    scan: Scan,
    status: FolderStatus,
}

/// Either an examined folder or a status that stops there.
enum Exam {
    Ready(Box<Examined>),
    Stopped(FolderStatus),
}

impl Engine {
    fn folder_ctx<'a>(
        &'a self,
        api: &'a crate::api::Api,
        link: &'a LinkedFolder,
        ctx: &'a OpContext,
    ) -> Result<FolderCtx<'a>> {
        Ok(FolderCtx {
            api,
            db: self.db(),
            link,
            rules: IgnoreRules::new(&link.ignore)?,
            ctx,
            cache_dir: &self.inner.cache_dir,
        })
    }

    /// Scans the folder and compares it with its base and the server head.
    async fn examine(&self, f: &FolderCtx<'_>) -> Result<Exam> {
        let link = f.link;
        let bare = |state| FolderStatus::bare(link.id, state, link.paused);
        if !link.local_path.is_dir() {
            return Ok(Exam::Stopped(bare(SyncState::MissingLocalPath)));
        }
        let base = f.base()?;
        let (info, head) = match f.remote(base.as_ref()).await {
            Ok(remote) => remote,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Exam::Stopped(bare(SyncState::RemoteMissing)));
            }
            Err(e) => return Err(e),
        };
        // This device's conflict snapshot: still open, or decided elsewhere (e.g. in
        // the web UI). Kept while it exists: it proves the local content is stored.
        let mut pending = f.pending_conflict()?;
        let mut open = false;
        if let Some(conflict) = &pending {
            match f.api.snapshot(&conflict.id, f.ctx).await? {
                Some(snapshot) => open = snapshot.conflict_resolved_at.is_none(),
                None => {
                    self.db().set_pending_conflict(link.id, None)?;
                    pending = None;
                }
            }
        }
        let scan = f.scan(base.as_ref().map(|b| &b.manifest)).await?;
        let comparison = compare(
            &scan.manifest,
            base.as_ref().map(|b| &b.manifest),
            head.as_ref().map(|h| &h.manifest),
            &f.rules,
        );
        let pending_open = pending.as_ref().is_some_and(|p| {
            // An open conflict matters while the local content is still that state and
            // differs from the head.
            open && !comparison.converged()
                && compare(
                    &scan.manifest,
                    Some(&p.manifest),
                    Some(&p.manifest),
                    &f.rules,
                )
                .converged()
        });
        let state = state_of(
            &comparison,
            !scan.problems.is_empty(),
            pending_open,
            info.unresolved_conflicts,
        );
        let status = FolderStatus {
            folder: link.id,
            state,
            paused: link.paused,
            local_changes: comparison.local.clone(),
            remote_changes: comparison.remote.clone(),
            conflicting: comparison.conflicting.clone(),
            problems: scan.problems.clone(),
            base_snapshot_id: base.as_ref().map(|b| b.id.clone()),
            head_snapshot_id: info.head_snapshot_id.clone(),
            unresolved_conflicts: info.unresolved_conflicts,
            pending_conflict_id: pending
                .as_ref()
                .filter(|_| pending_open)
                .map(|p| p.id.clone()),
        };
        Ok(Exam::Ready(Box::new(Examined {
            base,
            pending,
            head,
            scan,
            status,
        })))
    }

    /// Computes a folder's status (scans it and asks the server; changes nothing but
    /// the file index and the cached state).
    pub async fn status(&self, id: i64, ctx: &OpContext) -> Result<FolderStatus> {
        let _guard = self.lock(id).await?;
        let link = self.folder(id)?;
        let api = self.api()?;
        let f = self.folder_ctx(&api, &link, ctx)?;
        let status = match self.examine(&f).await? {
            Exam::Ready(examined) => examined.status,
            Exam::Stopped(status) => status,
        };
        self.db().set_last_state(id, status.state, None)?;
        Ok(status)
    }

    /// Brings a folder in sync (see the module docs). Refused while paused.
    pub async fn sync(&self, id: i64, ctx: &OpContext) -> Result<SyncReport> {
        let _guard = self.lock(id).await?;
        let link = self.folder(id)?;
        if link.paused {
            return Err(Error::Paused);
        }
        self.recorded(&link, "sync", self.sync_locked(&link, ctx, true).await)
    }

    /// "Keep this device's version": makes the conflict snapshot (default: this
    /// device's pending one) the head with a restore, then syncs. Unpauses the folder.
    pub async fn keep_local(
        &self,
        id: i64,
        conflict_id: Option<&str>,
        ctx: &OpContext,
    ) -> Result<SyncReport> {
        let _guard = self.lock(id).await?;
        let link = self.folder(id)?;
        let result = async {
            let api = self.api()?;
            let conflict = self.conflict_to_decide(&link, conflict_id)?;
            let info = api.folder(&link.folder_id, ctx).await?;
            let restored = api
                .restore(&conflict, info.head_snapshot_id.as_deref(), ctx)
                .await?;
            tracing::info!(folder = link.id, head = %restored.id, "kept the conflict version");
            if link.pending_conflict_id.as_deref() == Some(conflict.as_str()) {
                self.db().set_pending_conflict(link.id, None)?;
            }
            self.db().set_paused(link.id, false)?;
            let link = self.folder(id)?;
            self.sync_locked(&link, ctx, true).await
        }
        .await;
        self.recorded(&link, "keep_local", result)
    }

    /// "Keep the server's version": resolves the conflict snapshot (default: this
    /// device's pending one) and pulls the head. Local files are replaced only under the
    /// safety rule: content not on the server is first stored as a new conflict
    /// snapshot. Unpauses the folder.
    pub async fn keep_server(
        &self,
        id: i64,
        conflict_id: Option<&str>,
        ctx: &OpContext,
    ) -> Result<SyncReport> {
        let _guard = self.lock(id).await?;
        let link = self.folder(id)?;
        let result = async {
            let api = self.api()?;
            let conflict = self.conflict_to_decide(&link, conflict_id)?;
            let f = self.folder_ctx(&api, &link, ctx)?;
            // The pending conflict holds the local state: keep it for the safety check.
            let pending = f.pending_conflict()?;
            api.resolve(&conflict, ctx).await?;
            if link.pending_conflict_id.as_deref() == Some(conflict.as_str()) {
                self.db().set_pending_conflict(link.id, None)?;
            }
            self.db().set_paused(link.id, false)?;
            let link = self.folder(id)?;
            let f = self.folder_ctx(&api, &link, ctx)?;
            let examined = match self.examine(&f).await? {
                Exam::Ready(examined) => examined,
                Exam::Stopped(status) => {
                    return Ok(SyncReport {
                        outcome: SyncOutcome::UpToDate,
                        status,
                    });
                }
            };
            examined.scan.require_complete()?;
            let outcome = match &examined.head {
                Some(head) => {
                    let extra: Vec<&Snap> = pending.iter().collect();
                    self.pull_safely(&f, &examined, head, &extra, false).await?
                }
                None => SyncOutcome::UpToDate,
            };
            self.report(&f, outcome).await
        }
        .await;
        self.recorded(&link, "keep_server", result)
    }

    /// "Decide later": pauses the folder (no automatic sync) until a decision.
    pub fn decide_later(&self, id: i64) -> Result<LinkedFolder> {
        let link = self.set_paused(id, true)?;
        self.log(&NewActivity {
            folder_id: Some(id),
            folder_name: Some(&link.folder_name),
            action: "decide_later",
            outcome: "ok",
            ..NewActivity::default()
        });
        Ok(link)
    }

    fn conflict_to_decide(&self, link: &LinkedFolder, conflict_id: Option<&str>) -> Result<String> {
        conflict_id
            .map(str::to_owned)
            .or_else(|| link.pending_conflict_id.clone())
            .ok_or_else(|| Error::InvalidInput("no conflict snapshot to decide".to_owned()))
    }

    /// [`Engine::sync`] with the folder lock held.
    async fn sync_locked(
        &self,
        link: &LinkedFolder,
        ctx: &OpContext,
        retry: bool,
    ) -> Result<SyncReport> {
        let api = self.api()?;
        let f = self.folder_ctx(&api, link, ctx)?;
        let examined = match self.examine(&f).await? {
            Exam::Ready(examined) => examined,
            Exam::Stopped(status) => {
                return Ok(SyncReport {
                    outcome: SyncOutcome::UpToDate,
                    status,
                });
            }
        };
        let outcome = match examined.status.state {
            SyncState::Error => {
                return Err(Error::ScanProblems(examined.status.problems.clone()));
            }
            SyncState::InSync | SyncState::ServerConflict => {
                if let Some(head) = &examined.head
                    && examined.base.as_ref().map(|b| b.id.as_str()) != Some(head.id.as_str())
                {
                    self.db().set_base(link.id, &head.id, &head.stored)?;
                }
                if examined.pending.is_some() {
                    // Converged: the conflict no longer concerns this device.
                    self.db().set_pending_conflict(link.id, None)?;
                }
                SyncOutcome::UpToDate
            }
            SyncState::LocalChanges => match self.push(&f, &examined).await {
                Err(e) if retry && e.kind() == ErrorKind::Conflict => {
                    // The head moved meanwhile: look again (now a conflict, or a pull).
                    return Box::pin(self.sync_locked(link, ctx, false)).await;
                }
                other => other?,
            },
            SyncState::RemoteChanges => match &examined.head {
                Some(head) => self.pull_safely(&f, &examined, head, &[], true).await?,
                None => SyncOutcome::UpToDate,
            },
            SyncState::Conflict => self.store_conflict(&f, &examined).await?,
            SyncState::MissingLocalPath | SyncState::RemoteMissing => SyncOutcome::UpToDate,
        };
        self.report(&f, outcome).await
    }

    /// The outcome with a fresh status.
    async fn report(&self, f: &FolderCtx<'_>, outcome: SyncOutcome) -> Result<SyncReport> {
        let status = match self.examine(f).await? {
            Exam::Ready(examined) => examined.status,
            Exam::Stopped(status) => status,
        };
        self.db().set_last_state(f.link.id, status.state, None)?;
        Ok(SyncReport { outcome, status })
    }

    /// Records the result of an operation (activity log, last error).
    fn recorded(
        &self,
        link: &LinkedFolder,
        action: &str,
        result: Result<SyncReport>,
    ) -> Result<SyncReport> {
        match &result {
            Ok(report) => {
                let (files, bytes, snapshot) = match &report.outcome {
                    SyncOutcome::UpToDate => (0, 0, None),
                    SyncOutcome::Pushed {
                        snapshot_id,
                        uploaded_chunks,
                        uploaded_bytes,
                    } => (
                        *uploaded_chunks,
                        *uploaded_bytes,
                        Some(snapshot_id.as_str()),
                    ),
                    SyncOutcome::Pulled {
                        snapshot_id,
                        changes,
                        downloaded_bytes,
                    } => (*changes, *downloaded_bytes, Some(snapshot_id.as_str())),
                    SyncOutcome::NeedsDecision { conflict_id } => {
                        (0, 0, Some(conflict_id.as_str()))
                    }
                };
                let action = match (&report.outcome, action) {
                    (SyncOutcome::Pushed { .. }, "sync") => "push",
                    (SyncOutcome::Pulled { .. }, "sync") => "pull",
                    (SyncOutcome::NeedsDecision { .. }, "sync") => "conflict",
                    (SyncOutcome::UpToDate, "sync") => return result,
                    _ => action,
                };
                self.log(&NewActivity {
                    folder_id: Some(link.id),
                    folder_name: Some(&link.folder_name),
                    action,
                    outcome: "ok",
                    files,
                    bytes,
                    snapshot_id: snapshot,
                    ..NewActivity::default()
                });
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(folder = link.id, action, %message, "sync operation failed");
                if let Error::LocalChanged { path } = error {
                    // Its cached chunks may be stale (rewritten with the same size and
                    // mtime): hash it again at the next scan.
                    if let Err(e) = self.db().update_index(link.id, [], [path.as_str()]) {
                        tracing::warn!(error = %e, "could not update the file index");
                    }
                }
                if let Err(e) = self.db().set_last_state(
                    link.id,
                    link.last_state.unwrap_or(SyncState::Error),
                    Some(&message),
                ) {
                    tracing::warn!(error = %e, "could not record the error");
                }
                if error.kind() != ErrorKind::Cancelled {
                    self.log(&NewActivity {
                        folder_id: Some(link.id),
                        folder_name: Some(&link.folder_name),
                        action,
                        outcome: "error",
                        error_kind: Some(error.kind().as_str()),
                        error: Some(&message),
                        ..NewActivity::default()
                    });
                }
            }
        }
        result
    }

    /// Commits the local state as the new head.
    async fn push(&self, f: &FolderCtx<'_>, ex: &Examined) -> Result<SyncOutcome> {
        let head = ex.head.as_ref();
        let committed = push::commit(
            f,
            &ex.scan,
            head.map(|h| h.id.as_str()),
            head.map(|h| &h.manifest),
            SnapshotKind::Normal,
        )
        .await?;
        self.db()
            .set_base(f.link.id, &committed.snap.id, &committed.snap.stored)?;
        Ok(SyncOutcome::Pushed {
            snapshot_id: committed.snapshot.id,
            uploaded_chunks: committed.uploaded.chunks,
            uploaded_bytes: committed.uploaded.bytes,
        })
    }

    /// Stores the local state as a conflict snapshot (unless already done for this
    /// content) and asks for a decision.
    async fn store_conflict(&self, f: &FolderCtx<'_>, ex: &Examined) -> Result<SyncOutcome> {
        if let Some(pending) = &ex.pending
            && compare(
                &ex.scan.manifest,
                Some(&pending.manifest),
                Some(&pending.manifest),
                &f.rules,
            )
            .converged()
        {
            return Ok(SyncOutcome::NeedsDecision {
                conflict_id: pending.id.clone(),
            });
        }
        let committed = self.commit_conflict(f, ex).await?;
        self.db().set_pending_conflict(
            f.link.id,
            Some((committed.snap.id.as_str(), committed.snap.stored.as_slice())),
        )?;
        Ok(SyncOutcome::NeedsDecision {
            conflict_id: committed.snap.id,
        })
    }

    async fn commit_conflict(&self, f: &FolderCtx<'_>, ex: &Examined) -> Result<push::Committed> {
        push::commit(
            f,
            &ex.scan,
            ex.base.as_ref().map(|b| b.id.as_str()),
            ex.head.as_ref().map(|h| &h.manifest),
            SnapshotKind::Conflict,
        )
        .await
    }

    /// Applies `head`, first storing local content that isn't on the server (the safety
    /// rule). `resolve_copy`: resolve that safety copy after the pull (it only holds
    /// what the base had, when retention deleted the base snapshot).
    async fn pull_safely(
        &self,
        f: &FolderCtx<'_>,
        ex: &Examined,
        head: &Snap,
        extra: &[&Snap],
        resolve_copy: bool,
    ) -> Result<SyncOutcome> {
        let mut safe = SafeSet::default();
        safe.add(&head.manifest);
        for snap in ex
            .base
            .iter()
            .chain(ex.pending.iter())
            .chain(extra.iter().copied())
        {
            if f.api.snapshot(&snap.id, f.ctx).await?.is_some() {
                safe.add(&snap.manifest);
            }
        }
        let plan = plan_pull(&ex.scan.manifest, &ex.scan.files, &head.manifest, &f.rules);
        let mut safety_copy = None;
        let at_risk = unsafe_paths(&plan, &safe);
        if !at_risk.is_empty() {
            tracing::info!(
                folder = f.link.id,
                files = at_risk.len(),
                "storing local content on the server before replacing it"
            );
            // If this fails, nothing local has been touched.
            let committed = self.commit_conflict(f, ex).await?;
            safe.add(&committed.snap.manifest);
            safety_copy = Some(committed.snap.id);
        }
        let pulled = pull::apply_head(f, &ex.scan, head, &safe).await?;
        if let Some(copy) = safety_copy {
            if resolve_copy {
                f.api.resolve(&copy, f.ctx).await?;
            } else {
                self.log(&NewActivity {
                    folder_id: Some(f.link.id),
                    folder_name: Some(&f.link.folder_name),
                    action: "conflict",
                    outcome: "ok",
                    snapshot_id: Some(&copy),
                    ..NewActivity::default()
                });
            }
        }
        Ok(SyncOutcome::Pulled {
            snapshot_id: head.id.clone(),
            changes: pulled.changes,
            downloaded_bytes: pulled.downloaded_bytes,
        })
    }
}
