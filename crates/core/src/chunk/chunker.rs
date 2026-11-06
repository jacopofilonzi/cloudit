//! FastCDC content-defined chunking.
//!
//! Durable format (changing any of this changes every chunk boundary, so it needs a new
//! manifest format version):
//!
//! - algorithm: FastCDC 2020 as implemented by the `fastcdc` crate (`fastcdc::v2020`),
//!   normalization level 1, gear table seed 0 (the crate's default table);
//! - sizes: min [`MIN_CHUNK_SIZE`] (64 KiB), average [`AVG_CHUNK_SIZE`] (256 KiB), max
//!   [`MAX_CHUNK_SIZE`] (1 MiB);
//! - a file of at most `MIN_CHUNK_SIZE` bytes is a single chunk; an empty file has no chunks;
//!   every chunk but the last is at least `MIN_CHUNK_SIZE`, every chunk is at most
//!   `MAX_CHUNK_SIZE`;
//! - each chunk is hashed with SHA-256 of its bytes ([`ChunkHash`]).
//!
//! [`chunk_reader`] streams any [`Read`] holding at most one `MAX_CHUNK_SIZE` buffer in
//! memory, so multi-GB files are fine; [`chunk_slice`] is the in-memory equivalent and
//! yields the same boundaries. Golden vectors in the tests pin the boundaries and hashes.

use std::io::{self, Read};

use fastcdc::v2020::{FastCDC, Normalization, StreamCDC};

use super::{AVG_CHUNK_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE};
use crate::error::ChunkerError;
use crate::hash::ChunkHash;

const LEVEL: Normalization = Normalization::Level1;
const SEED: u64 = 0;

/// A chunk read from a stream: its hash and its (uncompressed) bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub hash: ChunkHash,
    /// Byte offset of the chunk in the source.
    pub offset: u64,
    pub data: Vec<u8>,
}

impl Chunk {
    /// Chunk length in bytes (never 0, at most [`MAX_CHUNK_SIZE`]).
    #[must_use]
    pub fn len(&self) -> u32 {
        // A chunk never exceeds MAX_CHUNK_SIZE (1 MiB), so it always fits.
        u32::try_from(self.data.len()).unwrap_or(u32::MAX)
    }

    /// Always false: the chunker never yields empty chunks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// A chunk of an in-memory buffer, borrowing its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceChunk<'a> {
    pub hash: ChunkHash,
    /// Byte offset of the chunk in the buffer.
    pub offset: usize,
    pub data: &'a [u8],
}

/// Streaming chunker over a [`Read`]; see [`chunk_reader`].
pub struct Chunker<R: Read> {
    inner: StreamCDC<RetryInterrupted<R>>,
}

impl<R: Read> Iterator for Chunker<R> {
    type Item = Result<Chunk, ChunkerError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.next()?;
        Some(
            next.map(|c| Chunk {
                hash: ChunkHash::of(&c.data),
                offset: c.offset,
                data: c.data,
            })
            .map_err(|e| ChunkerError(io::Error::from(e))),
        )
    }
}

/// Splits everything `source` yields into chunks, reading it incrementally.
///
/// Memory use is bounded by one [`MAX_CHUNK_SIZE`] buffer plus the chunk being returned.
/// Read errors (other than [`io::ErrorKind::Interrupted`], which is retried) end the
/// iteration with an error item.
pub fn chunk_reader<R: Read>(source: R) -> Chunker<R> {
    Chunker {
        inner: StreamCDC::with_level_and_seed(
            RetryInterrupted(source),
            MIN_CHUNK_SIZE as usize,
            AVG_CHUNK_SIZE as usize,
            MAX_CHUNK_SIZE as usize,
            LEVEL,
            SEED,
        ),
    }
}

/// Splits an in-memory buffer into chunks; same boundaries as [`chunk_reader`].
pub fn chunk_slice(data: &[u8]) -> impl Iterator<Item = SliceChunk<'_>> {
    FastCDC::with_level_and_seed(
        data,
        MIN_CHUNK_SIZE as usize,
        AVG_CHUNK_SIZE as usize,
        MAX_CHUNK_SIZE as usize,
        LEVEL,
        SEED,
    )
    .filter_map(move |c| {
        let bytes = data.get(c.offset..c.offset + c.length)?;
        Some(SliceChunk {
            hash: ChunkHash::of(bytes),
            offset: c.offset,
            data: bytes,
        })
    })
}

/// `fastcdc` propagates `Interrupted` as a hard error; retry it like `read_exact` does.
struct RetryInterrupted<R>(R);

impl<R: Read> Read for RetryInterrupted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.0.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::pseudo_random;

    fn reader_chunks(data: &[u8]) -> Vec<(u64, u32, ChunkHash)> {
        chunk_reader(data)
            .map(|c| c.map(|c| (c.offset, c.len(), c.hash)).unwrap())
            .collect()
    }

    fn slice_chunks(data: &[u8]) -> Vec<(u64, u32, ChunkHash)> {
        chunk_slice(data)
            .map(|c| (c.offset as u64, c.data.len() as u32, c.hash))
            .collect()
    }

    #[test]
    fn empty_input_has_no_chunks() {
        assert!(reader_chunks(&[]).is_empty());
        assert!(slice_chunks(&[]).is_empty());
    }

    #[test]
    fn small_input_is_one_chunk() {
        for len in [1, 1000, MIN_CHUNK_SIZE as usize] {
            let data = pseudo_random(1, len);
            let chunks = reader_chunks(&data);
            assert_eq!(
                chunks,
                vec![(0, len as u32, ChunkHash::of(&data))],
                "len {len}"
            );
            assert_eq!(slice_chunks(&data), chunks);
        }
    }

    #[test]
    fn boundaries_respect_limits_and_cover_input() {
        let data = pseudo_random(7, 6 * 1024 * 1024 + 123);
        let chunks = reader_chunks(&data);
        assert!(chunks.len() > 10);
        let mut expected_offset = 0;
        for (i, (offset, len, hash)) in chunks.iter().enumerate() {
            assert_eq!(*offset, expected_offset);
            assert!(*len <= MAX_CHUNK_SIZE);
            if i + 1 < chunks.len() {
                assert!(*len >= MIN_CHUNK_SIZE, "chunk {i} is {len} bytes");
            }
            let start = *offset as usize;
            assert_eq!(*hash, ChunkHash::of(&data[start..start + *len as usize]));
            expected_offset += u64::from(*len);
        }
        assert_eq!(expected_offset, data.len() as u64);
    }

    #[test]
    fn zero_filled_input_cuts_at_max() {
        // No content-defined cut point in uniform data: every chunk is max-sized.
        let data = vec![0_u8; 3 * MAX_CHUNK_SIZE as usize + 10];
        let lens: Vec<u32> = reader_chunks(&data).iter().map(|c| c.1).collect();
        assert_eq!(
            lens,
            vec![MAX_CHUNK_SIZE, MAX_CHUNK_SIZE, MAX_CHUNK_SIZE, 10]
        );
    }

    #[test]
    fn deterministic_and_reader_equals_slice() {
        let data = pseudo_random(42, 3 * 1024 * 1024);
        let a = reader_chunks(&data);
        assert_eq!(a, reader_chunks(&data));
        assert_eq!(a, slice_chunks(&data));
    }

    /// A reader that returns at most 1000 bytes per call and an `Interrupted` error every
    /// other call: the result must not depend on how the source delivers bytes.
    struct Trickle<'a> {
        data: &'a [u8],
        interrupt: bool,
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = buf.len().min(1000).min(self.data.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    #[test]
    fn short_reads_and_interrupts_do_not_change_boundaries() {
        let data = pseudo_random(3, 2 * 1024 * 1024 + 5);
        let trickled: Vec<_> = chunk_reader(Trickle {
            data: &data,
            interrupt: false,
        })
        .map(|c| c.map(|c| (c.offset, c.len(), c.hash)).unwrap())
        .collect();
        assert_eq!(trickled, slice_chunks(&data));
    }

    #[test]
    fn read_error_is_reported() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk on fire"))
            }
        }
        let first = chunk_reader(Failing).next().unwrap();
        assert!(first.is_err());
    }

    #[test]
    fn inserting_a_byte_reuses_most_chunks() {
        let data = pseudo_random(99, 8 * 1024 * 1024);
        let mut shifted = Vec::with_capacity(data.len() + 1);
        shifted.push(0xAB);
        shifted.extend_from_slice(&data);
        let before: std::collections::HashSet<ChunkHash> =
            slice_chunks(&data).iter().map(|c| c.2).collect();
        let after = slice_chunks(&shifted);
        let reused = after.iter().filter(|c| before.contains(&c.2)).count();
        assert!(after.len() > 20);
        // Only the chunk(s) around the insertion point may change.
        assert!(
            reused + 2 >= after.len(),
            "only {reused} of {} chunks reused",
            after.len()
        );
    }

    /// Golden vector: pins the chunking parameters and algorithm. If this fails, the
    /// durable chunk format changed: do not update the expected values, bump the format.
    #[test]
    fn golden_boundaries_and_hashes() {
        let data = pseudo_random(2024, 2 * 1024 * 1024);
        let got: Vec<(u64, u32, String)> = reader_chunks(&data)
            .into_iter()
            .map(|(o, l, h)| (o, l, h.to_string()))
            .collect();
        let expected: Vec<(u64, u32, String)> = GOLDEN
            .iter()
            .map(|&(o, l, h)| (o, l, h.to_owned()))
            .collect();
        assert_eq!(got, expected);
    }

    /// `(offset, length, sha256)` of `pseudo_random(2024, 2 MiB)`.
    const GOLDEN: &[(u64, u32, &str)] = &[
        (
            0,
            300_046,
            "d6155a340f76b5a6baf47153fa307c5c59ad10dd09d5336bed6bd3d6544f4e32",
        ),
        (
            300_046,
            381_024,
            "0b7e39eea7664da4fa6198090288a8537845525fc381a3c9df083c69379e902b",
        ),
        (
            681_070,
            177_060,
            "fa6a5c554956fdbf590615001cc9a40a4baf18cc0cd68323159db81b2b47c1c7",
        ),
        (
            858_130,
            151_178,
            "10a8ad092db8c35709488fc949a226f24853faf73d78e7a4cd11b2b1a865d9c3",
        ),
        (
            1_009_308,
            213_701,
            "1d6f5f22994874c2fef1eb56d405faf5b4af55d1d71b1c7cc750d6692ea4b126",
        ),
        (
            1_223_009,
            320_293,
            "adf0aa4fb279c9f39b4994bccfba291079fded2cdb9e65bb38409492adc6485b",
        ),
        (
            1_543_302,
            553_850,
            "dbd50f2460952ba658df6eb211f23a4c5362db1005e9bbeb7a40c0807d04fba2",
        ),
    ];
}
