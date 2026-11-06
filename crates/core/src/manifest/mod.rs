//! The manifest: the complete description of one snapshot of a synced folder.
//!
//! - [`model`]: the types and the JSON schema;
//! - [`codec`]: canonical JSON, hashing and zstd compression;
//! - [`validate`]: the rules every stored manifest satisfies;
//!
//! A manifest is immutable and content-addressed by its [`crate::hash::ManifestHash`].

pub mod codec;
pub mod model;
pub mod validate;

pub use codec::{
    DEFAULT_MAX_MANIFEST_BYTES, EncodedManifest, decode_manifest, from_canonical_json,
};
pub use model::{ChunkRef, EntryKind, MANIFEST_VERSION, Manifest, ManifestEntry, Timestamp};
