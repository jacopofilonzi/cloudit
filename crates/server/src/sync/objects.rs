//! The protocol between writers of sync objects (uploads, adoption of presigned
//! uploads, manifest storage) and the collectors that delete them (`jobs::gc`,
//! `jobs::orphans`, `jobs::trash_purge`).
//!
//! # The invariant
//!
//! A `chunks` / `manifests` row always has its object, and a snapshot only references
//! rows that exist. The database enforces the second part (foreign keys from
//! `snapshot_chunks` and `snapshots`); this module the first, across the database and
//! blob storage, which share no transaction.
//!
//! # The rule: one lock per scope, the scope row
//!
//! [`lock_scope`] (`UPDATE scopes SET stored_bytes = stored_bytes WHERE id = ?`) is the
//! first statement of every transaction that
//!
//! - **inserts** a row (`usage::insert_charged`): under the lock it checks that the
//!   object is in place ([`ensure_present`]: `HEAD`; a proxy upload stores it again if
//!   it is gone, an adoption gives up), then inserts and commits;
//! - **deletes objects** of a scope (GC phase 2, orphan sweep): under the lock it
//!   re-checks that no row exists for the object (and, for orphans, that the object is
//!   still older than the grace period), then deletes the object and commits.
//!
//! The garbage collector deletes **rows** in a separate, earlier transaction (also
//! under the lock), which records a tombstone (`object_deletions`) for each deleted row
//! in the same transaction, so a crash never forgets an object; phase 2 deletes the
//! objects of the tombstones and the tombstones. A scope purge deletes all rows of the
//! scope with the scope row itself, then deletes the scope's objects by listing.
//!
//! # Why no interleaving breaks the invariant
//!
//! An object deletion and a row insertion for the same key are serialized by the lock
//! (on PostgreSQL a row lock held to the end of the transaction, on SQLite the database
//! write lock):
//!
//! - deletion first: it commits only if no row existed (checked under the lock); the
//!   writer then takes the lock, sees the object missing and stores it again (proxy
//!   upload, manifest) or reports the chunk missing (adoption: the client uploads it
//!   again), so it never inserts a row for a missing object;
//! - insertion first: the deletion then sees the row and keeps the object.
//!
//! A writer that stored its object *before* a deletion that removed it is covered by
//! the first case: the check under the lock happens after the deletion committed. A
//! deletion that crashes after deleting objects but before committing leaves no row
//! (rows were deleted in phase 1, or never existed for orphans) and its tombstones
//! stay for the next run. A purge holds the scope row lock while it deletes the scope
//! row: a writer waiting for the lock then finds no scope ([`lock_scope`] is `false`)
//! and inserts nothing; objects it stored are deleted by the purge's listing or, if
//! stored after it, by the orphan sweep.
//!
//! Snapshot side: a commit checks its rows before its transaction; if GC deletes one
//! meanwhile, the foreign key fails and the commit answers `chunk_missing` (the client
//! uploads again) or stores the manifest again. On PostgreSQL a GC delete racing a
//! commit that already inserted references fails on the foreign key and is retried
//! at the next run. Retention and restore lock the snapshot row (`jobs::retention`).
//!
//! Assumptions: an object delete or put takes effect before the request returns (no
//! delayed delete after the deleting transaction ended); a presigned client that
//! replaces an object with different bytes of the same size within the same second is
//! not detected by the adoption re-check (it can only damage its own scope, and
//! downloads verify every chunk's hash).
//!
//! # Cost
//!
//! Each new row costs one extra `HEAD` inside a transaction that holds the scope lock,
//! so the row inserts of one scope are serialized (on SQLite every write is serialized
//! anyway). Object puts and verification happen outside the lock; uploads of different
//! scopes don't contend on PostgreSQL. The collectors hold a scope's lock while they
//! delete at most [`DELETE_BATCH`] objects ([`DELETE_CONCURRENCY`] at a time).

use anyhow::Context as _;
use bytes::Bytes;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use uuid::Uuid;

use crate::db::entities::{chunks, manifests, scopes};
use crate::storage::{BlobMeta, BlobStore};

/// Objects deleted per locked transaction.
pub const DELETE_BATCH: usize = 100;
/// Object deletes in flight at once.
pub const DELETE_CONCURRENCY: usize = 16;

/// The two kinds of sync objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Chunk,
    Manifest,
}

impl Kind {
    /// The stored name (`object_deletions.kind`) and key segment prefix.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chunk => "chunk",
            Self::Manifest => "manifest",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "chunk" => Some(Self::Chunk),
            "manifest" => Some(Self::Manifest),
            _ => None,
        }
    }

    /// The blob key of an object of this kind (`hash` already validated).
    #[must_use]
    pub fn key(self, scope: Uuid, hash: &str) -> String {
        match self {
            Self::Chunk => format!("scopes/{scope}/chunks/{hash}"),
            Self::Manifest => format!("scopes/{scope}/manifests/{hash}"),
        }
    }
}

/// Parses a blob key of the sync layout: `(scope, kind, hash)`; `None` for anything
/// else (temporary files, foreign objects).
#[must_use]
pub fn parse_key(key: &str) -> Option<(Uuid, Kind, String)> {
    let mut parts = key.split('/');
    let (Some("scopes"), Some(scope), Some(dir), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    let scope = scope.parse().ok()?;
    let kind = match dir {
        "chunks" => {
            hash.parse::<cloudit_core::hash::ChunkHash>().ok()?;
            Kind::Chunk
        }
        "manifests" => {
            hash.parse::<cloudit_core::hash::ManifestHash>().ok()?;
            Kind::Manifest
        }
        _ => return None,
    };
    Some((scope, kind, hash.to_owned()))
}

/// Takes the scope lock (see the module docs): must be the first statement of the
/// transaction. `false` if the scope doesn't exist (purged).
pub async fn lock_scope(db: &impl ConnectionTrait, scope: Uuid) -> anyhow::Result<bool> {
    let result = scopes::Entity::update_many()
        .col_expr(
            scopes::Column::StoredBytes,
            Expr::col(scopes::Column::StoredBytes),
        )
        .filter(scopes::Column::Id.eq(scope))
        .exec(db)
        .await
        .context("locking a scope")?;
    Ok(result.rows_affected == 1)
}

/// Whether the scope has a row for the object.
pub async fn row_exists(
    db: &impl ConnectionTrait,
    scope: Uuid,
    kind: Kind,
    hash: &str,
) -> anyhow::Result<bool> {
    let id = (scope, hash.to_owned());
    Ok(match kind {
        Kind::Chunk => chunks::Entity::find_by_id(id).one(db).await?.is_some(),
        Kind::Manifest => manifests::Entity::find_by_id(id).one(db).await?.is_some(),
    })
}

/// What a writer knows about the object it stored.
#[derive(Debug, Clone, Copy)]
pub enum Stored<'a> {
    /// The verified bytes (proxy upload, manifest): stored again if missing.
    Bytes(&'a Bytes),
    /// An object verified where it is (adoption): it must still be that object.
    Verified(&'a BlobMeta),
}

impl Stored<'_> {
    #[must_use]
    pub fn size(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Verified(meta) => meta.size,
        }
    }
}

/// Under the scope lock: makes sure the object is in place; `false` if it is gone (or
/// replaced) and can't be restored (adoption).
pub async fn ensure_present(
    blobs: &BlobStore,
    key: &str,
    stored: Stored<'_>,
) -> anyhow::Result<bool> {
    let current = blobs.head(key).await?;
    match stored {
        Stored::Bytes(bytes) => {
            if current.is_none_or(|meta| meta.size != bytes.len() as u64) {
                blobs.put(key, bytes.clone()).await?;
            }
            Ok(true)
        }
        Stored::Verified(expected) => Ok(current.is_some_and(|meta| {
            meta.size == expected.size && meta.last_modified == expected.last_modified
        })),
    }
}
