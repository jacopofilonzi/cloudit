//! `cloudit-core`: the shared sync engine used by both the server and the desktop client.
//!
//! It owns everything both sides must agree on byte-for-byte:
//!
//! - [`protocol`]: the HTTP API request/response types. They are the single source of
//!   truth for the TypeScript models too: `make types` exports them through `ts-rs`
//!   into `packages/api-types/src/generated/`.
//! - [`chunk`]: content-defined chunking (FastCDC) and the stored chunk encoding.
//! - [`hash`]: [`hash::ChunkHash`] / [`hash::ManifestHash`] (SHA-256, lowercase hex).
//! - [`manifest`]: the snapshot description (canonical JSON, hash, zstd), its validation
//!   and diff.
//! - [`path`]: validation of the relative paths a manifest may contain.
//!
//! The chunking parameters, the chunk encoding, the manifest format and the hashing are
//! **durable formats** (stored on servers): each module's top-level doc is their spec, and
//! golden-vector tests pin them. Changing them requires a new format version.
//!
//! The crate does no I/O on its own: callers bring the storage, the network and the file
//! system (chunking reads from any `std::io::Read`).

pub mod chunk;
pub mod error;
pub mod hash;
pub mod protocol;

#[cfg(test)]
mod testutil;
