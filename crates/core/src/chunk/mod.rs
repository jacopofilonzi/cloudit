//! Content-defined chunking and the stored chunk encoding.
//!
//! - [`chunker`]: splits a file into chunks with FastCDC (durable parameters below).
//! - [`codec`]: the byte format a chunk is stored in (header byte + raw or zstd payload).
//!
//! A chunk is identified by [`crate::hash::ChunkHash`], the SHA-256 of its uncompressed bytes.

pub mod codec;

/// Minimum chunk size in bytes (64 KiB). Only the last chunk of a file can be smaller.
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
/// Target average chunk size in bytes (256 KiB).
pub const AVG_CHUNK_SIZE: u32 = 256 * 1024;
/// Maximum chunk size in bytes (1 MiB). Also the bound for decoding a stored chunk.
pub const MAX_CHUNK_SIZE: u32 = 1024 * 1024;
