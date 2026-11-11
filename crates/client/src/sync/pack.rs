//! Which chunks travel in chunk packs (`cloudit_core::pack`, servers at API level 2).
//!
//! Chunks smaller than [`PACK_CHUNK_LIMIT`] (uncompressed) are grouped into packs of at
//! most [`PACK_TARGET_BYTES`] uncompressed bytes and `MAX_PACK_RECORDS` records; larger
//! ones keep their own request (proxy or presigned). The stored form of a chunk is at
//! most its size + 1 byte (`encode_chunk` keeps zstd only when smaller), so a pack body
//! stays well under `MAX_PACK_BYTES`. Packs are only a transport: every chunk is still
//! verified against its hash on both sides, and the server stores each one on its own.

use cloudit_core::pack::MAX_PACK_RECORDS;

/// Chunks below this uncompressed size go in packs.
pub const PACK_CHUNK_LIMIT: u32 = 256 * 1024;
/// Uncompressed bytes of one pack, at most.
pub const PACK_TARGET_BYTES: u64 = 8 * 1024 * 1024;

/// Splits `items` into packs (in order) and the chunks sent one by one.
pub fn group<T>(items: Vec<T>, size: impl Fn(&T) -> u32) -> (Vec<Vec<T>>, Vec<T>) {
    let mut packs: Vec<Vec<T>> = Vec::new();
    let mut singles = Vec::new();
    let mut current: Vec<T> = Vec::new();
    let mut current_bytes = 0u64;
    for item in items {
        let len = size(&item);
        if len >= PACK_CHUNK_LIMIT {
            singles.push(item);
            continue;
        }
        if current.len() == MAX_PACK_RECORDS || current_bytes + u64::from(len) > PACK_TARGET_BYTES {
            packs.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes += u64::from(len);
        current.push(item);
    }
    if !current.is_empty() {
        packs.push(current);
    }
    (packs, singles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_chunks_are_grouped_within_the_limits() {
        let (packs, singles) = group(
            vec![10u32, PACK_CHUNK_LIMIT, 20, PACK_CHUNK_LIMIT - 1],
            |s| *s,
        );
        assert_eq!(packs, vec![vec![10, 20, PACK_CHUNK_LIMIT - 1]]);
        assert_eq!(singles, vec![PACK_CHUNK_LIMIT]);

        let (packs, _) = group(vec![1u32; 2500], |s| *s);
        assert_eq!(
            packs.iter().map(Vec::len).collect::<Vec<_>>(),
            [1000, 1000, 500]
        );

        let big = PACK_CHUNK_LIMIT - 1;
        let (packs, _) = group(vec![big; 70], |s| *s);
        for pack in &packs {
            assert!(pack.iter().map(|s| u64::from(*s)).sum::<u64>() <= PACK_TARGET_BYTES);
        }
        assert_eq!(packs.iter().map(Vec::len).sum::<usize>(), 70);
        assert_eq!(packs.len(), 3);

        let (packs, singles) = group(Vec::<u32>::new(), |s| *s);
        assert!(packs.is_empty() && singles.is_empty());
    }
}
