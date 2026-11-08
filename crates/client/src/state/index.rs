//! The `file_index` table: a per-folder cache of each file's chunks keyed by size and
//! mtime, so unchanged files are not hashed again. Chunk lists are stored as packed
//! records of 32 hash bytes + 4 size bytes (little endian).

use std::collections::HashMap;

use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, Timestamp};
use rusqlite::params;

use super::Db;
use crate::error::Result;

/// One cached file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub size: u64,
    pub mtime: Timestamp,
    pub executable: bool,
    pub chunks: Vec<ChunkRef>,
}

const RECORD: usize = 36;

fn pack(chunks: &[ChunkRef]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunks.len() * RECORD);
    for chunk in chunks {
        out.extend_from_slice(chunk.hash.as_bytes());
        out.extend_from_slice(&chunk.size.to_le_bytes());
    }
    out
}

fn unpack(bytes: &[u8]) -> Option<Vec<ChunkRef>> {
    if !bytes.len().is_multiple_of(RECORD) {
        return None;
    }
    bytes
        .as_chunks::<RECORD>()
        .0
        .iter()
        .map(|record| {
            let (hash, size) = record.split_at(32);
            Some(ChunkRef {
                hash: ChunkHash::from_bytes(hash.try_into().ok()?),
                size: u32::from_le_bytes(size.try_into().ok()?),
            })
        })
        .collect()
}

impl Db {
    /// The cached files of a folder (corrupt rows are skipped: they get re-hashed).
    pub fn load_index(&self, folder: i64) -> Result<HashMap<String, IndexEntry>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT path, size, mtime_secs, mtime_nanos, executable, chunks
                 FROM file_index WHERE folder_id = ?1",
            )?;
            let rows = stmt.query_map([folder], |r| {
                let size: i64 = r.get(1)?;
                let nanos: i64 = r.get(3)?;
                let chunks: Vec<u8> = r.get(5)?;
                Ok((
                    r.get::<_, String>(0)?,
                    u64::try_from(size).ok(),
                    r.get::<_, i64>(2)?,
                    u32::try_from(nanos).ok(),
                    r.get::<_, bool>(4)?,
                    unpack(&chunks),
                ))
            })?;
            let mut index = HashMap::new();
            for row in rows {
                if let (path, Some(size), secs, Some(nanos), executable, Some(chunks)) = row? {
                    index.insert(
                        path,
                        IndexEntry {
                            size,
                            mtime: Timestamp { secs, nanos },
                            executable,
                            chunks,
                        },
                    );
                }
            }
            Ok(index)
        })
    }

    /// Replaces a folder's whole index.
    pub fn replace_index<'a>(
        &self,
        folder: i64,
        entries: impl IntoIterator<Item = (&'a str, &'a IndexEntry)>,
    ) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM file_index WHERE folder_id = ?1", [folder])?;
            insert_entries(&tx, folder, entries)?;
            tx.commit()
        })
    }

    /// Inserts or replaces some entries and removes `removed`.
    pub fn update_index<'a>(
        &self,
        folder: i64,
        entries: impl IntoIterator<Item = (&'a str, &'a IndexEntry)>,
        removed: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            {
                let mut delete =
                    tx.prepare("DELETE FROM file_index WHERE folder_id = ?1 AND path = ?2")?;
                for path in removed {
                    delete.execute(params![folder, path])?;
                }
            }
            insert_entries(&tx, folder, entries)?;
            tx.commit()
        })
    }
}

fn insert_entries<'a>(
    tx: &rusqlite::Transaction<'_>,
    folder: i64,
    entries: impl IntoIterator<Item = (&'a str, &'a IndexEntry)>,
) -> rusqlite::Result<()> {
    let mut insert = tx.prepare(
        "INSERT OR REPLACE INTO file_index
            (folder_id, path, size, mtime_secs, mtime_nanos, executable, chunks)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (path, entry) in entries {
        insert.execute(params![
            folder,
            path,
            i64::try_from(entry.size).unwrap_or(i64::MAX),
            entry.mtime.secs,
            entry.mtime.nanos,
            entry.executable,
            pack(&entry.chunks),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_lists_round_trip() {
        let chunks = vec![
            ChunkRef {
                hash: ChunkHash::of(b"a"),
                size: 1,
            },
            ChunkRef {
                hash: ChunkHash::of(b"bb"),
                size: 70_000,
            },
        ];
        assert_eq!(unpack(&pack(&chunks)).unwrap(), chunks);
        assert!(unpack(&[1, 2, 3]).is_none());
    }
}
