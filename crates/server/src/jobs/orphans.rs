//! `orphan_sweep` (daily): deletes objects under `scopes/*/chunks/` and
//! `scopes/*/manifests/` that have no row and were last modified more than `GC_GRACE`
//! ago: uploads through presigned URLs never committed, objects left by a crash
//! between storing and inserting the row, invalid uploads, leftovers of a purge.
//!
//! Lists every object, keeps the old ones whose key matches the layout, drops those
//! with a row, then per scope under the scope lock (`sync::objects`) checks again that
//! there is no row and that the object (`HEAD`) is still older than the grace period
//! (a re-upload refreshes it), and deletes it. Objects of a scope that no longer exists
//! are deleted without the lock (no row can appear for them). Keys outside the layout
//! (e.g. temporary files of the filesystem store) are never touched.

use std::collections::BTreeMap;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use sea_orm::TransactionTrait as _;
use uuid::Uuid;

use super::gc::delete_objects;
use super::{Counts, RunContext};
use crate::hooks::{self, Point};
use crate::state::AppState;
use crate::sync::objects::{self, Kind};

/// Old objects examined at a time.
const BATCH: usize = 500;

/// An old object without a known row.
#[derive(Debug, Clone)]
struct Orphan {
    kind: Kind,
    hash: String,
    key: String,
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    let grace = chrono::TimeDelta::from_std(ctx.gc_grace).context("GC grace")?;
    let cutoff = crate::time::now() - grace;
    let mut listing = state.blobs.list("scopes")?;
    let mut batch: BTreeMap<Uuid, Vec<Orphan>> = BTreeMap::new();
    let mut pending = 0;
    while let Some(meta) = listing.next().await {
        let meta = meta?;
        counts.add("objects_listed", 1);
        let Some((scope, kind, hash)) = objects::parse_key(&meta.key) else {
            counts.add("unrecognized", 1);
            continue;
        };
        if meta.last_modified >= cutoff {
            continue;
        }
        batch.entry(scope).or_default().push(Orphan {
            kind,
            hash,
            key: meta.key,
        });
        pending += 1;
        if pending >= BATCH {
            ctx.check()?;
            sweep(state, std::mem::take(&mut batch), cutoff, counts).await?;
            pending = 0;
        }
    }
    sweep(state, batch, cutoff, counts).await
}

async fn sweep(
    state: &AppState,
    batch: BTreeMap<Uuid, Vec<Orphan>>,
    cutoff: DateTime<Utc>,
    counts: &mut Counts,
) -> anyhow::Result<()> {
    for (scope, items) in batch {
        let mut orphans = Vec::new();
        for item in items {
            if !objects::row_exists(&state.db, scope, item.kind, &item.hash).await? {
                orphans.push(item);
            }
        }
        if orphans.is_empty() {
            continue;
        }
        hooks::reached(Point::SweepListed, &scope.to_string()).await;
        for chunk in orphans.chunks(objects::DELETE_BATCH) {
            counts.add(
                "objects_deleted",
                sweep_locked(state, scope, chunk, cutoff).await?,
            );
        }
    }
    Ok(())
}

/// Under the scope lock: deletes those of `orphans` still without row and still old.
async fn sweep_locked(
    state: &AppState,
    scope: Uuid,
    orphans: &[Orphan],
    cutoff: DateTime<Utc>,
) -> anyhow::Result<u64> {
    let tx = state.db.begin().await.context("starting a transaction")?;
    let locked = objects::lock_scope(&tx, scope).await?;
    let mut keys = Vec::new();
    for orphan in orphans {
        if locked && objects::row_exists(&tx, scope, orphan.kind, &orphan.hash).await? {
            continue;
        }
        match state.blobs.head(&orphan.key).await? {
            Some(meta) if meta.last_modified < cutoff => keys.push(orphan.key.clone()),
            _ => {}
        }
    }
    delete_objects(state, &keys).await?;
    tx.commit().await.context("committing an orphan sweep")?;
    if !keys.is_empty() {
        tracing::info!(scope_id = %scope, deleted = keys.len(), "orphan objects deleted");
    }
    Ok(keys.len() as u64)
}
