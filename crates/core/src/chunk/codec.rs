//! Stored chunk encoding: how a chunk's bytes are written to blob storage.
//!
//! Durable format: `[header: u8][payload]`.
//!
//! | header | payload |
//! | --- | --- |
//! | `0x00` | the raw chunk bytes |
//! | `0x01` | one zstd frame of the chunk bytes |
//!
//! Any other header is rejected. [`encode_chunk`] compresses with zstd level
//! [`ZSTD_LEVEL`] and keeps the result only if it is strictly smaller than the raw bytes
//! (already-compressed data such as images or archives is stored raw). The compressed bytes
//! are not part of the chunk's identity (the [`ChunkHash`] is over the uncompressed bytes),
//! so a different zstd version producing different frames is harmless.
//!
//! [`decode_chunk`] never trusts the stored bytes: it bounds the decompressed size to
//! [`MAX_CHUNK_SIZE`] (zip-bomb protection; the bound is enforced while streaming, nothing
//! larger is ever allocated) and verifies the SHA-256 against the expected hash.

use std::io::Read;

use super::MAX_CHUNK_SIZE;
use crate::error::ChunkError;
use crate::hash::ChunkHash;

/// Header byte: the payload is the raw chunk.
pub const ENCODING_RAW: u8 = 0x00;
/// Header byte: the payload is a zstd frame.
pub const ENCODING_ZSTD: u8 = 0x01;
/// zstd level used by [`encode_chunk`].
pub const ZSTD_LEVEL: i32 = 3;

const MAX: usize = MAX_CHUNK_SIZE as usize;

/// Encodes a chunk for storage (header byte + raw or zstd payload).
///
/// # Errors
///
/// [`ChunkError::TooLarge`] if `data` exceeds [`MAX_CHUNK_SIZE`] (it could never be decoded),
/// [`ChunkError::Compress`] if zstd fails.
pub fn encode_chunk(data: &[u8]) -> Result<Vec<u8>, ChunkError> {
    if data.len() > MAX {
        return Err(ChunkError::TooLarge(data.len()));
    }
    let compressed = zstd::bulk::compress(data, ZSTD_LEVEL).map_err(ChunkError::Compress)?;
    let (header, payload) = if compressed.len() < data.len() {
        (ENCODING_ZSTD, compressed.as_slice())
    } else {
        (ENCODING_RAW, data)
    };
    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(header);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Decodes a stored chunk and checks that its content hashes to `expected`.
///
/// # Errors
///
/// [`ChunkError::MissingHeader`], [`ChunkError::UnknownEncoding`], [`ChunkError::TooLarge`]
/// (decoded size above [`MAX_CHUNK_SIZE`]), [`ChunkError::Decompress`] (corrupt zstd frame) or
/// [`ChunkError::HashMismatch`].
pub fn decode_chunk(stored: &[u8], expected: &ChunkHash) -> Result<Vec<u8>, ChunkError> {
    let (&header, payload) = stored.split_first().ok_or(ChunkError::MissingHeader)?;
    let data = match header {
        ENCODING_RAW => {
            if payload.len() > MAX {
                return Err(ChunkError::TooLarge(payload.len()));
            }
            payload.to_vec()
        }
        ENCODING_ZSTD => decompress_bounded(payload)?,
        other => return Err(ChunkError::UnknownEncoding(other)),
    };
    let actual = ChunkHash::of(&data);
    if actual != *expected {
        return Err(ChunkError::HashMismatch {
            expected: *expected,
            actual,
        });
    }
    Ok(data)
}

fn decompress_bounded(payload: &[u8]) -> Result<Vec<u8>, ChunkError> {
    let decoder =
        zstd::stream::read::Decoder::with_buffer(payload).map_err(ChunkError::Decompress)?;
    let mut out = Vec::new();
    // Read one byte past the limit to tell "exactly MAX" from "more than MAX".
    decoder
        .take(MAX as u64 + 1)
        .read_to_end(&mut out)
        .map_err(ChunkError::Decompress)?;
    if out.len() > MAX {
        return Err(ChunkError::TooLarge(out.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::pseudo_random;

    #[test]
    fn compressible_round_trips_as_zstd() {
        let data = b"save game data ".repeat(1000);
        let stored = encode_chunk(&data).unwrap();
        assert_eq!(stored[0], ENCODING_ZSTD);
        assert!(stored.len() < data.len());
        assert_eq!(decode_chunk(&stored, &ChunkHash::of(&data)).unwrap(), data);
    }

    #[test]
    fn incompressible_falls_back_to_raw() {
        let data = pseudo_random(5, 64 * 1024);
        let stored = encode_chunk(&data).unwrap();
        assert_eq!(stored[0], ENCODING_RAW);
        assert_eq!(&stored[1..], data.as_slice());
        assert_eq!(decode_chunk(&stored, &ChunkHash::of(&data)).unwrap(), data);
    }

    #[test]
    fn max_size_chunk_round_trips() {
        let data = vec![7_u8; MAX];
        let stored = encode_chunk(&data).unwrap();
        assert_eq!(decode_chunk(&stored, &ChunkHash::of(&data)).unwrap(), data);
        let raw = [&[ENCODING_RAW][..], &data].concat();
        assert_eq!(decode_chunk(&raw, &ChunkHash::of(&data)).unwrap(), data);
    }

    #[test]
    fn empty_chunk_round_trips() {
        let stored = encode_chunk(&[]).unwrap();
        assert_eq!(stored, vec![ENCODING_RAW]);
        assert_eq!(
            decode_chunk(&stored, &ChunkHash::of(&[])).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn rejects_oversized_input() {
        let data = vec![0_u8; MAX + 1];
        assert!(matches!(encode_chunk(&data), Err(ChunkError::TooLarge(_))));
        let raw = [&[ENCODING_RAW][..], &data].concat();
        let err = decode_chunk(&raw, &ChunkHash::of(&data)).unwrap_err();
        assert!(matches!(err, ChunkError::TooLarge(n) if n == MAX + 1));
    }

    #[test]
    fn rejects_zip_bomb() {
        // 8 MiB of zeros compresses to a few hundred bytes; decoding must stop right after MAX.
        let bomb = zstd::bulk::compress(&vec![0_u8; 8 * 1024 * 1024], 3).unwrap();
        let stored = [&[ENCODING_ZSTD][..], &bomb].concat();
        let err = decode_chunk(&stored, &ChunkHash::of(b"")).unwrap_err();
        assert!(
            matches!(err, ChunkError::TooLarge(n) if n == MAX + 1),
            "{err:?}"
        );
        assert_eq!(err.code(), "chunk_too_large");
    }

    #[test]
    fn rejects_corrupt_payload() {
        let data = b"hello hello hello hello hello hello hello".repeat(100);
        let mut stored = encode_chunk(&data).unwrap();
        assert_eq!(stored[0], ENCODING_ZSTD);
        let len = stored.len();
        stored.truncate(len - 5);
        let err = decode_chunk(&stored, &ChunkHash::of(&data)).unwrap_err();
        assert!(matches!(err, ChunkError::Decompress(_)), "{err:?}");
        let garbage = [ENCODING_ZSTD, 1, 2, 3, 4, 5, 6, 7, 8];
        let err = decode_chunk(&garbage, &ChunkHash::of(&data)).unwrap_err();
        assert!(matches!(err, ChunkError::Decompress(_)), "{err:?}");
    }

    #[test]
    fn rejects_wrong_hash() {
        let stored = encode_chunk(b"abc").unwrap();
        let err = decode_chunk(&stored, &ChunkHash::of(b"abd")).unwrap_err();
        match err {
            ChunkError::HashMismatch { expected, actual } => {
                assert_eq!(expected, ChunkHash::of(b"abd"));
                assert_eq!(actual, ChunkHash::of(b"abc"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_header_and_missing_header() {
        let err = decode_chunk(&[0x02, 1, 2], &ChunkHash::of(b"")).unwrap_err();
        assert!(matches!(err, ChunkError::UnknownEncoding(0x02)));
        let err = decode_chunk(&[], &ChunkHash::of(b"")).unwrap_err();
        assert!(matches!(err, ChunkError::MissingHeader));
    }
}
