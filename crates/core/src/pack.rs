//! Chunk packs: many stored chunks in one HTTP body, so that a folder of thousands of
//! small files doesn't cost one request per chunk.
//!
//! **A pack is a wire format only, never stored.** The server unpacks an uploaded pack
//! and stores every chunk as its own object (`scopes/{scope}/chunks/{hash}`) with its
//! own row, exactly like a single upload; a downloaded pack is assembled from those
//! objects on the fly. Nothing about the durable formats (chunking, the stored chunk
//! encoding, manifests, hashes) depends on it.
//!
//! # Format (version 1)
//!
//! Content type [`PACK_CONTENT_TYPE`]. The body is:
//!
//! ```text
//! [version u8 = 0x01]
//! [hash: 32 bytes][length: u32 little-endian][length bytes]   (record, repeated)
//! ```
//!
//! until the end of the body (no count, no trailer). `hash` is the raw SHA-256 of the
//! chunk's uncompressed bytes ([`ChunkHash`]); the bytes are the chunk in the stored
//! chunk encoding (`chunk::codec`), as stored on the server. A record is never split:
//! a body that ends inside a record is invalid.
//!
//! [`decode_pack`] (the same rules for uploads and downloads) rejects: an empty body
//! (no version byte), any version but 1, a truncated record, a length of 0 or over
//! [`MAX_STORED_CHUNK_BYTES`], more than [`MAX_PACK_RECORDS`] records, a body over
//! [`MAX_PACK_BYTES`] and a hash listed twice. A body of only the version byte is a
//! valid pack with no records. It only splits the body: callers still decode every
//! chunk with `decode_chunk` against its hash before using or storing it.
//!
//! A future version changes the first byte; decoders refuse versions they don't know.

use std::collections::HashSet;
use std::ops::Range;

use crate::error::PackError;
use crate::hash::ChunkHash;
use crate::protocol::MAX_STORED_CHUNK_BYTES;

/// Content type of a pack body (both directions).
pub const PACK_CONTENT_TYPE: &str = "application/vnd.cloudit.chunk-pack";
/// The pack format version written by [`PackWriter`].
pub const PACK_VERSION: u8 = 1;
/// Most records in one pack.
pub const MAX_PACK_RECORDS: usize = 1000;
/// Largest pack body (version byte and record headers included).
pub const MAX_PACK_BYTES: usize = 16 * 1024 * 1024;
/// Bytes of a record before its chunk bytes (hash + length).
pub const RECORD_HEADER_BYTES: usize = 32 + 4;

/// One record of a decoded pack: the hash and where its stored chunk bytes are in the
/// body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    pub hash: ChunkHash,
    pub range: Range<usize>,
}

/// The header of one record (for writers that stream the chunk bytes after it).
#[must_use]
pub fn record_header(hash: &ChunkHash, len: u32) -> [u8; RECORD_HEADER_BYTES] {
    let mut out = [0u8; RECORD_HEADER_BYTES];
    out[..32].copy_from_slice(hash.as_bytes());
    out[32..].copy_from_slice(&len.to_le_bytes());
    out
}

/// Splits a pack body into its records (rules in the module docs).
///
/// # Errors
///
/// [`PackError`] for any violation of the format.
pub fn decode_pack(body: &[u8]) -> Result<Vec<PackEntry>, PackError> {
    if body.len() > MAX_PACK_BYTES {
        return Err(PackError::TooLarge(body.len()));
    }
    let (&version, mut rest) = body.split_first().ok_or(PackError::MissingVersion)?;
    if version != PACK_VERSION {
        return Err(PackError::UnsupportedVersion(version));
    }
    let mut offset = 1usize;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    while !rest.is_empty() {
        if entries.len() == MAX_PACK_RECORDS {
            return Err(PackError::TooManyRecords(MAX_PACK_RECORDS));
        }
        let (header, after) = rest
            .split_at_checked(RECORD_HEADER_BYTES)
            .ok_or(PackError::Truncated(offset))?;
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&header[..32]);
        let hash = ChunkHash::from_bytes(raw);
        let mut len = [0u8; 4];
        len.copy_from_slice(&header[32..]);
        let len =
            usize::try_from(u32::from_le_bytes(len)).map_err(|_| PackError::Truncated(offset))?;
        if len == 0 || len > MAX_STORED_CHUNK_BYTES {
            return Err(PackError::BadLength { hash, len });
        }
        if after.len() < len {
            return Err(PackError::Truncated(offset));
        }
        if !seen.insert(hash) {
            return Err(PackError::Duplicate(hash));
        }
        let start = offset + RECORD_HEADER_BYTES;
        entries.push(PackEntry {
            hash,
            range: start..start + len,
        });
        offset = start + len;
        rest = &after[len..];
    }
    Ok(entries)
}

/// Builds a pack body in memory.
#[derive(Debug, Clone)]
pub struct PackWriter {
    buf: Vec<u8>,
    records: usize,
}

impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PackWriter {
    /// An empty pack (just the version byte).
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: vec![PACK_VERSION],
            records: 0,
        }
    }

    /// Records written so far.
    #[must_use]
    pub const fn records(&self) -> usize {
        self.records
    }

    /// The body size so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether no record was written.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records == 0
    }

    /// Whether one more record of `len` stored bytes still fits the limits.
    #[must_use]
    pub fn fits(&self, len: usize) -> bool {
        self.records < MAX_PACK_RECORDS
            && self.buf.len() + RECORD_HEADER_BYTES + len <= MAX_PACK_BYTES
    }

    /// Appends one record. Duplicates are not checked here (the decoder refuses them).
    ///
    /// # Errors
    ///
    /// [`PackError::BadLength`] for an empty or oversized chunk, [`PackError::TooLarge`]
    /// / [`PackError::TooManyRecords`] when the pack is full (see [`Self::fits`]).
    pub fn push(&mut self, hash: &ChunkHash, stored: &[u8]) -> Result<(), PackError> {
        if stored.is_empty() || stored.len() > MAX_STORED_CHUNK_BYTES {
            return Err(PackError::BadLength {
                hash: *hash,
                len: stored.len(),
            });
        }
        if self.records == MAX_PACK_RECORDS {
            return Err(PackError::TooManyRecords(MAX_PACK_RECORDS));
        }
        if !self.fits(stored.len()) {
            return Err(PackError::TooLarge(
                self.buf.len() + RECORD_HEADER_BYTES + stored.len(),
            ));
        }
        let len = u32::try_from(stored.len()).map_err(|_| PackError::BadLength {
            hash: *hash,
            len: stored.len(),
        })?;
        self.buf.extend_from_slice(&record_header(hash, len));
        self.buf.extend_from_slice(stored);
        self.records += 1;
        Ok(())
    }

    /// The finished body.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::codec::{decode_chunk, encode_chunk};

    fn hash(n: u8) -> ChunkHash {
        ChunkHash::of(&[n])
    }

    #[test]
    fn round_trip() {
        let chunks: Vec<Vec<u8>> = (0..5u8)
            .map(|n| vec![n; 1000 * usize::from(n) + 1])
            .collect();
        let mut writer = PackWriter::new();
        for data in &chunks {
            writer
                .push(&ChunkHash::of(data), &encode_chunk(data).unwrap())
                .unwrap();
        }
        assert_eq!(writer.records(), 5);
        let body = writer.finish();
        let entries = decode_pack(&body).unwrap();
        assert_eq!(entries.len(), 5);
        for (entry, data) in entries.iter().zip(&chunks) {
            assert_eq!(entry.hash, ChunkHash::of(data));
            assert_eq!(
                &decode_chunk(&body[entry.range.clone()], &entry.hash).unwrap(),
                data
            );
        }
    }

    #[test]
    fn the_layout_is_pinned() {
        let mut writer = PackWriter::new();
        writer.push(&hash(1), &[0x00, 0xaa, 0xbb]).unwrap();
        let body = writer.finish();
        let mut expected = vec![0x01];
        expected.extend_from_slice(hash(1).as_bytes());
        expected.extend_from_slice(&[3, 0, 0, 0, 0x00, 0xaa, 0xbb]);
        assert_eq!(body, expected);
        assert_eq!(decode_pack(&[0x01]).unwrap(), Vec::new());
    }

    #[test]
    fn hostile_bodies_are_rejected() {
        assert!(matches!(decode_pack(&[]), Err(PackError::MissingVersion)));
        assert!(matches!(
            decode_pack(&[2]),
            Err(PackError::UnsupportedVersion(2))
        ));

        let mut writer = PackWriter::new();
        writer.push(&hash(1), &[0, 1, 2, 3]).unwrap();
        let body = writer.finish();
        for cut in 2..body.len() {
            assert!(
                matches!(decode_pack(&body[..cut]), Err(PackError::Truncated(1))),
                "cut at {cut}"
            );
        }

        let mut oversized = vec![1];
        oversized.extend_from_slice(&record_header(&hash(1), u32::MAX));
        assert!(matches!(
            decode_pack(&oversized),
            Err(PackError::BadLength { .. })
        ));
        let mut empty = vec![1];
        empty.extend_from_slice(&record_header(&hash(1), 0));
        assert!(matches!(
            decode_pack(&empty),
            Err(PackError::BadLength { len: 0, .. })
        ));

        let mut dup = vec![1];
        for _ in 0..2 {
            dup.extend_from_slice(&record_header(&hash(1), 1));
            dup.push(0);
        }
        assert!(matches!(decode_pack(&dup), Err(PackError::Duplicate(_))));

        let mut many = vec![1];
        for n in 0..=MAX_PACK_RECORDS {
            let h = ChunkHash::of(&n.to_le_bytes());
            many.extend_from_slice(&record_header(&h, 1));
            many.push(0);
        }
        assert!(matches!(
            decode_pack(&many),
            Err(PackError::TooManyRecords(_))
        ));

        let huge = vec![1u8; MAX_PACK_BYTES + 1];
        assert!(matches!(decode_pack(&huge), Err(PackError::TooLarge(_))));
    }

    #[test]
    fn the_writer_stops_at_the_limits() {
        let mut writer = PackWriter::new();
        for n in 0..MAX_PACK_RECORDS {
            writer.push(&ChunkHash::of(&n.to_le_bytes()), &[0]).unwrap();
        }
        assert!(!writer.fits(1));
        assert!(matches!(
            writer.push(&hash(1), &[0]),
            Err(PackError::TooManyRecords(_))
        ));

        let big = vec![0u8; MAX_STORED_CHUNK_BYTES];
        let mut writer = PackWriter::new();
        let mut n = 0u32;
        while writer.fits(big.len()) {
            writer.push(&ChunkHash::of(&n.to_le_bytes()), &big).unwrap();
            n += 1;
        }
        assert!(writer.push(&hash(1), &big).is_err());
        assert!(writer.len() <= MAX_PACK_BYTES);
        assert!(decode_pack(&writer.finish()).is_ok());
        assert!(PackWriter::new().push(&hash(1), &[]).is_err());
    }
}
