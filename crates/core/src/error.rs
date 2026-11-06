//! Error types of `cloudit-core`.
//!
//! Each enum variant is a stable error kind: callers (the server mapping a rejected upload
//! to an API error code, the client deciding whether to retry) match on variants, and
//! [`ChunkError::code`] / [`ManifestError::code`] / [`PathError::code`] give a stable
//! snake_case string for logs and API bodies. Never rename or reuse a code.

use thiserror::Error;

use crate::hash::ChunkHash;

/// A hash string that is not exactly 64 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HashParseError {
    #[error("hash must be 64 hex characters, got {0} bytes")]
    Length(usize),
    #[error("hash contains {0:?}: only lowercase hex (0-9, a-f) is allowed")]
    InvalidChar(char),
}

/// Why a relative path was rejected (see [`crate::path`] for the rules).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("path is empty")]
    Empty,
    #[error("path is {0} bytes, longer than the limit")]
    TooLong(usize),
    #[error("path segment is {0} bytes, longer than the limit")]
    SegmentTooLong(usize),
    #[error("path is absolute or has a drive/UNC prefix")]
    Absolute,
    #[error("path contains a backslash; the separator is '/'")]
    Backslash,
    #[error("path has an empty segment (leading, trailing or doubled '/')")]
    EmptySegment,
    #[error("path has a '.' or '..' segment")]
    DotSegment,
    #[error("path contains a control character")]
    ControlChar,
    #[error("path contains {0:?}, which is invalid on Windows")]
    InvalidChar(char),
    #[error("path segment ends with a space or a dot")]
    TrailingSpaceOrDot,
    #[error("path segment {0:?} is a reserved device name on Windows")]
    ReservedName(String),
}

impl PathError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Empty => "path_empty",
            Self::TooLong(_) => "path_too_long",
            Self::SegmentTooLong(_) => "path_segment_too_long",
            Self::Absolute => "path_absolute",
            Self::Backslash => "path_backslash",
            Self::EmptySegment => "path_empty_segment",
            Self::DotSegment => "path_dot_segment",
            Self::ControlChar => "path_control_char",
            Self::InvalidChar(_) => "path_invalid_char",
            Self::TrailingSpaceOrDot => "path_trailing_space_or_dot",
            Self::ReservedName(_) => "path_reserved_name",
        }
    }
}

/// Reading a source while chunking it failed.
#[derive(Debug, Error)]
#[error("cannot read the data to chunk")]
pub struct ChunkerError(#[from] pub std::io::Error);

/// Encoding or decoding a stored chunk failed.
#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("stored chunk is empty (no encoding header)")]
    MissingHeader,
    #[error("unknown chunk encoding header 0x{0:02x}")]
    UnknownEncoding(u8),
    #[error("chunk is larger than the maximum chunk size ({0} bytes or more)")]
    TooLarge(usize),
    #[error("zstd compression failed")]
    Compress(#[source] std::io::Error),
    #[error("zstd payload is corrupt")]
    Decompress(#[source] std::io::Error),
    #[error("chunk content hash {actual} does not match the expected {expected}")]
    HashMismatch {
        expected: ChunkHash,
        actual: ChunkHash,
    },
}

impl ChunkError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingHeader => "chunk_missing_header",
            Self::UnknownEncoding(_) => "chunk_unknown_encoding",
            Self::TooLarge(_) => "chunk_too_large",
            Self::Compress(_) => "chunk_compress_failed",
            Self::Decompress(_) => "chunk_corrupt",
            Self::HashMismatch { .. } => "chunk_hash_mismatch",
        }
    }
}

/// Encoding, decoding or validating a manifest failed.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("zstd compression failed")]
    Compress(#[source] std::io::Error),
    #[error("zstd payload is corrupt")]
    Decompress(#[source] std::io::Error),
    #[error("decompressed manifest exceeds {0} bytes")]
    TooLarge(usize),
    #[error("manifest is not valid JSON for this format")]
    Json(#[source] serde_json::Error),
    #[error("manifest JSON is not in canonical form")]
    NonCanonical,
    #[error("unsupported manifest version {0}")]
    UnsupportedVersion(u64),
    #[error("invalid path {path:?}")]
    InvalidPath {
        path: String,
        #[source]
        source: PathError,
    },
    #[error("entries are not sorted: {path:?} comes after {previous:?}")]
    NotSorted { previous: String, path: String },
    #[error("duplicate entry {0:?}")]
    Duplicate(String),
    #[error("{a:?} and {b:?} differ only by letter case")]
    CaseCollision { a: String, b: String },
    #[error("{child:?} is inside {parent:?}, which is a file")]
    ParentIsFile { parent: String, child: String },
    #[error("{child:?} is inside {parent:?}, which is listed as an empty directory")]
    DirHasChildren { parent: String, child: String },
    #[error("directory entry {0:?} has a size, chunks or the executable flag")]
    DirWithContent(String),
    #[error("chunk sizes of {path:?} sum to {chunks_total}, but the file size is {size}")]
    SizeMismatch {
        path: String,
        size: u64,
        chunks_total: u64,
    },
    #[error("{path:?} has a chunk of {size} bytes, outside 1..=maximum chunk size")]
    ChunkSizeOutOfRange { path: String, size: u32 },
    #[error("{path:?} has an invalid mtime (nanos must be below 1e9)")]
    InvalidMtime { path: String },
    #[error("chunk {hash} is listed with different sizes")]
    ChunkSizeConflict { hash: ChunkHash },
}

impl ManifestError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Compress(_) => "manifest_compress_failed",
            Self::Decompress(_) => "manifest_corrupt",
            Self::TooLarge(_) => "manifest_too_large",
            Self::Json(_) => "manifest_invalid_json",
            Self::NonCanonical => "manifest_non_canonical",
            Self::UnsupportedVersion(_) => "manifest_unsupported_version",
            Self::InvalidPath { .. } => "manifest_invalid_path",
            Self::NotSorted { .. } => "manifest_not_sorted",
            Self::Duplicate(_) => "manifest_duplicate_path",
            Self::CaseCollision { .. } => "manifest_case_collision",
            Self::ParentIsFile { .. } => "manifest_parent_is_file",
            Self::DirHasChildren { .. } => "manifest_dir_has_children",
            Self::DirWithContent(_) => "manifest_dir_with_content",
            Self::SizeMismatch { .. } => "manifest_size_mismatch",
            Self::ChunkSizeOutOfRange { .. } => "manifest_chunk_size_out_of_range",
            Self::InvalidMtime { .. } => "manifest_invalid_mtime",
            Self::ChunkSizeConflict { .. } => "manifest_chunk_size_conflict",
        }
    }
}
