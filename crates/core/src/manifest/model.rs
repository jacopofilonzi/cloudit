//! Manifest types and their JSON schema (format version 1).
//!
//! ```json
//! {"version":1,"entries":[
//!   {"path":"saves/slot1.sav","kind":"file","size":70000,
//!    "mtime":{"secs":1700000000,"nanos":123456789},"executable":false,
//!    "chunks":[{"hash":"<64 lowercase hex>","size":65536},{"hash":"…","size":4464}]},
//!   {"path":"screenshots","kind":"dir","size":0,"mtime":{"secs":1700000000,"nanos":0},
//!    "executable":false,"chunks":[]}
//! ]}
//! ```
//!
//! - `version`: always [`MANIFEST_VERSION`] (1) for this schema.
//! - `entries`: sorted by `path` in byte order of its UTF-8 string, no duplicates.
//! - `path`: relative, `/`-separated, validated by [`crate::path::validate_path`].
//! - `kind`: `"file"` or `"dir"`. Directories appear only when **empty**: a non-empty
//!   directory is implied by the paths of its contents. Symlinks and other special files are
//!   not represented (the scanner skips them).
//! - `size`: file length in bytes (u64); always 0 for a dir.
//! - `mtime`: last modification time as seconds since the Unix epoch (i64, negative before
//!   1970) plus nanoseconds (u32, `0..1_000_000_000`). Split in two so both fit exactly in a
//!   JavaScript number and any OS precision (Windows 100 ns, Linux 1 ns) round-trips.
//! - `executable`: the Unix executable bit (any of `u+x`/`g+x`/`o+x`); always false for a dir.
//! - `chunks`: the file content in order, as `{hash, size}`; empty for a dir and for an empty
//!   file. Sizes sum to `size`; each is in `1..=MAX_CHUNK_SIZE`.
//!
//! All fields are always present; unknown fields are rejected.

use serde::{Deserialize, Serialize};

use crate::hash::ChunkHash;

/// The manifest format version this crate reads and writes.
pub const MANIFEST_VERSION: u32 = 1;

/// One snapshot of a folder. Build it with [`Manifest::new`] (sorts and validates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct Manifest {
    pub version: u32,
    pub entries: Vec<ManifestEntry>,
}

/// A file or an empty directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ManifestEntry {
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: Timestamp,
    pub executable: bool,
    pub chunks: Vec<ChunkRef>,
}

/// Entry kind. `Dir` is used only for empty directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum EntryKind {
    File,
    Dir,
}

/// Modification time: `secs` since the Unix epoch plus `nanos` (`0..1_000_000_000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

/// One chunk of a file: its hash and uncompressed length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub struct ChunkRef {
    pub hash: ChunkHash,
    pub size: u32,
}

impl ManifestEntry {
    /// A file entry; `size` is the sum of the chunk sizes.
    #[must_use]
    pub fn file(
        path: impl Into<String>,
        mtime: Timestamp,
        executable: bool,
        chunks: Vec<ChunkRef>,
    ) -> Self {
        let size = chunks.iter().map(|c| u64::from(c.size)).sum();
        Self {
            path: path.into(),
            kind: EntryKind::File,
            size,
            mtime,
            executable,
            chunks,
        }
    }

    /// An empty-directory entry.
    #[must_use]
    pub fn dir(path: impl Into<String>, mtime: Timestamp) -> Self {
        Self {
            path: path.into(),
            kind: EntryKind::Dir,
            size: 0,
            mtime,
            executable: false,
            chunks: Vec::new(),
        }
    }

    /// True if `other` has the same content (kind, size, chunks) and executable flag; the
    /// mtime is ignored.
    #[must_use]
    pub fn same_content(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.size == other.size
            && self.executable == other.executable
            && self.chunks == other.chunks
    }
}

impl Manifest {
    /// Builds a version-1 manifest: sorts `entries` by path and validates the result.
    ///
    /// # Errors
    ///
    /// Any [`crate::error::ManifestError`] validation failure.
    pub fn new(mut entries: Vec<ManifestEntry>) -> Result<Self, crate::error::ManifestError> {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Self {
            version: MANIFEST_VERSION,
            entries,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Looks an entry up by exact path (entries are sorted, so this is a binary search).
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&ManifestEntry> {
        self.entries
            .binary_search_by(|e| e.path.as_str().cmp(path))
            .ok()
            .and_then(|i| self.entries.get(i))
    }
}
