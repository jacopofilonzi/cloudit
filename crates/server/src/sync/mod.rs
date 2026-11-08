//! The sync protocol, server side: chunks, manifests, snapshots, usage and quotas
//! (the routes call these; access checks are `scopes::access`).
//!
//! # Object layout (blob storage)
//!
//! - `scopes/{scope_id}/chunks/{chunk_hash}`: a chunk in the stored chunk encoding of
//!   `cloudit_core::chunk::codec` (header byte + raw or zstd);
//! - `scopes/{scope_id}/manifests/{manifest_hash}`: a manifest in its stored form
//!   (`cloudit_core::manifest::codec`: one zstd frame of the canonical JSON).
//!
//! Dedup is per scope only, so an editor of a shared scope never writes into the
//! namespace of the owner's other scopes ([`keys`]).
//!
//! # Invariants
//!
//! - **A row means a verified object.** A `chunks` row is inserted only after its
//!   object was stored and decoded with `cloudit-core` to the expected hash; a
//!   `manifests` row only after the manifest decoded and validated. Order: object
//!   first, row second. An object without a row (a crash in between, an upload through
//!   a presigned URL never committed, an invalid upload) is an orphan the garbage
//!   collector removes after its grace period.
//! - **Usage is counted in the same transaction** as the row: `INSERT … ON CONFLICT DO
//!   NOTHING`, and only when a row was inserted `scopes.stored_bytes += stored_size`
//!   ([`usage::insert_charged`]). Concurrent duplicate uploads count once.
//! - A commit moves the head with one conditional UPDATE (`… WHERE id = ? AND
//!   head_snapshot_id = parent`, or `IS NULL`), in the transaction that inserts the
//!   snapshot and its `snapshot_chunks`; 0 rows → rollback, `conflict`.
//! - Snapshots never change except `pinned`, `label` and `conflict_resolved_at`; a
//!   restore inserts a copy.
//!
//! # Deleting (the jobs)
//!
//! - Writers and collectors follow the scope-lock protocol of [`objects`]: a row is
//!   inserted only under the scope lock after checking its object is in place, and an
//!   object is deleted only under the same lock after checking it has no row.
//! - The garbage collector (`jobs::gc`) deletes unreferenced rows older than
//!   `GC_GRACE` (default 24 h), decrementing `scopes.stored_bytes` in the same
//!   transaction, then their objects; [`usage::recompute`] gives the expected value.
//! - A chunk is referenced by `snapshot_chunks (scope_id, hash)` (foreign key: deleting
//!   a referenced row fails), a manifest by `snapshots (scope_id, manifest_hash)`. A
//!   commit that loses a row to GC answers `chunk_missing` and the client uploads
//!   again.
//! - The orphan sweep (`jobs::orphans`) deletes objects without row older than the
//!   grace period; retention (`jobs::retention`) never deletes the head, pinned
//!   snapshots or unresolved conflicts, and deletes a snapshot's `snapshot_chunks`
//!   first.

pub mod archive;
pub mod chunks;
pub mod keys;
pub mod manifests;
pub mod objects;
pub mod snapshots;
pub mod transfer;
pub mod usage;
