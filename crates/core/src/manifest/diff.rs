//! Comparing manifests, and chunk-set helpers.
//!
//! Diff semantics (by exact path; renames show up as removed + added):
//!
//! - **added**: path only in `new`; **removed**: path only in `old`;
//! - **modified**: in both, and kind, size, chunk list or executable flag differ
//!   ([`ManifestEntry::same_content`] is false); the mtime may differ too;
//! - **metadata only**: in both, same content, only the mtime differs. Nothing needs to be
//!   transferred, but a client may want to restore the mtime;
//! - **unchanged**: identical entries.
//!
//! Every list is in path byte order. The chunk helpers return deterministic (sorted) results
//! without duplicates.

use std::collections::{BTreeMap, BTreeSet};

use super::model::{EntryKind, Manifest, ManifestEntry};
use crate::hash::ChunkHash;

/// An entry present in both manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryChange<'a> {
    pub old: &'a ManifestEntry,
    pub new: &'a ManifestEntry,
}

/// Result of [`diff`]; see the module docs for the categories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestDiff<'a> {
    pub added: Vec<&'a ManifestEntry>,
    pub removed: Vec<&'a ManifestEntry>,
    pub modified: Vec<EntryChange<'a>>,
    pub metadata_only: Vec<EntryChange<'a>>,
    pub unchanged: Vec<&'a ManifestEntry>,
}

impl ManifestDiff<'_> {
    /// True if the two manifests describe the same content (mtime-only changes ignored).
    #[must_use]
    pub fn is_content_equal(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// Compares `old` with `new` by path.
#[must_use]
pub fn diff<'a>(old: &'a Manifest, new: &'a Manifest) -> ManifestDiff<'a> {
    // BTreeMap instead of a merge walk: correct even on an unvalidated, unsorted manifest.
    let old_by_path: BTreeMap<&str, &ManifestEntry> =
        old.entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let new_by_path: BTreeMap<&str, &ManifestEntry> =
        new.entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut out = ManifestDiff::default();
    for (path, &old_entry) in &old_by_path {
        match new_by_path.get(path) {
            None => out.removed.push(old_entry),
            Some(&new_entry) => {
                let change = EntryChange {
                    old: old_entry,
                    new: new_entry,
                };
                if !old_entry.same_content(new_entry) {
                    out.modified.push(change);
                } else if old_entry.mtime != new_entry.mtime {
                    out.metadata_only.push(change);
                } else {
                    out.unchanged.push(new_entry);
                }
            }
        }
    }
    out.added = new_by_path
        .iter()
        .filter(|(path, _)| !old_by_path.contains_key(*path))
        .map(|(_, &e)| e)
        .collect();
    out
}

impl Manifest {
    /// Every chunk hash referenced by the manifest, sorted, without duplicates.
    #[must_use]
    pub fn chunk_hashes(&self) -> BTreeSet<ChunkHash> {
        self.entries
            .iter()
            .flat_map(|e| e.chunks.iter().map(|c| c.hash))
            .collect()
    }

    /// The referenced chunks for which `is_available` is false, sorted, without duplicates.
    /// Pass e.g. `|h| stored.contains(h)` with a set of hashes already in storage.
    #[must_use]
    pub fn missing_chunks(&self, is_available: impl Fn(&ChunkHash) -> bool) -> Vec<ChunkHash> {
        self.chunk_hashes()
            .into_iter()
            .filter(|h| !is_available(h))
            .collect()
    }

    /// Sum of the file sizes (the folder's logical size).
    #[must_use]
    pub fn logical_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .fold(0_u64, |acc, e| acc.saturating_add(e.size))
    }

    /// Sum of the sizes of the distinct chunks (uncompressed): the content left after
    /// deduplication within this manifest.
    #[must_use]
    pub fn unique_chunk_size(&self) -> u64 {
        let unique: BTreeMap<ChunkHash, u32> = self
            .entries
            .iter()
            .flat_map(|e| e.chunks.iter().map(|c| (c.hash, c.size)))
            .collect();
        unique
            .values()
            .fold(0_u64, |acc, &s| acc.saturating_add(u64::from(s)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::manifest::model::{ChunkRef, Timestamp};

    const T0: Timestamp = Timestamp {
        secs: 100,
        nanos: 0,
    };
    const T1: Timestamp = Timestamp {
        secs: 200,
        nanos: 5,
    };

    fn c(n: u8, size: u32) -> ChunkRef {
        ChunkRef {
            hash: ChunkHash::of(&[n]),
            size,
        }
    }

    fn f(path: &str, t: Timestamp, chunks: Vec<ChunkRef>) -> ManifestEntry {
        ManifestEntry::file(path, t, false, chunks)
    }

    fn paths(entries: &[&ManifestEntry]) -> Vec<String> {
        entries.iter().map(|e| e.path.clone()).collect()
    }

    fn change_paths(changes: &[EntryChange<'_>]) -> Vec<String> {
        changes.iter().map(|c| c.new.path.clone()).collect()
    }

    #[test]
    fn diff_categories() {
        let old = Manifest::new(vec![
            f("same", T0, vec![c(1, 10)]),
            f("touched", T0, vec![c(2, 10)]),
            f("edited", T0, vec![c(3, 10)]),
            f("resized", T0, vec![c(4, 10)]),
            f("chmod", T0, vec![c(5, 10)]),
            f("became_dir", T0, vec![]),
            f("gone", T0, vec![c(6, 10)]),
        ])
        .unwrap();
        let mut chmod = f("chmod", T0, vec![c(5, 10)]);
        chmod.executable = true;
        let new = Manifest::new(vec![
            f("same", T0, vec![c(1, 10)]),
            f("touched", T1, vec![c(2, 10)]),
            f("edited", T1, vec![c(7, 10)]),
            f("resized", T0, vec![c(4, 10), c(8, 1)]),
            chmod,
            ManifestEntry::dir("became_dir", T0),
            f("new", T1, vec![c(1, 10)]),
        ])
        .unwrap();
        let d = diff(&old, &new);
        assert_eq!(paths(&d.added), ["new"]);
        assert_eq!(paths(&d.removed), ["gone"]);
        assert_eq!(
            change_paths(&d.modified),
            ["became_dir", "chmod", "edited", "resized"]
        );
        assert_eq!(change_paths(&d.metadata_only), ["touched"]);
        assert_eq!(paths(&d.unchanged), ["same"]);
        assert!(!d.is_content_equal());
    }

    #[test]
    fn mtime_only_is_content_equal() {
        let old = Manifest::new(vec![f("a", T0, vec![c(1, 10)])]).unwrap();
        let new = Manifest::new(vec![f("a", T1, vec![c(1, 10)])]).unwrap();
        let d = diff(&old, &new);
        assert!(d.is_content_equal());
        assert_eq!(d.metadata_only.len(), 1);
        assert!(diff(&old, &old).metadata_only.is_empty());
    }

    #[test]
    fn diff_against_empty() {
        let empty = Manifest::new(vec![]).unwrap();
        let m = Manifest::new(vec![f("a", T0, vec![]), f("b", T0, vec![])]).unwrap();
        assert_eq!(paths(&diff(&empty, &m).added), ["a", "b"]);
        assert_eq!(paths(&diff(&m, &empty).removed), ["a", "b"]);
    }

    #[test]
    fn chunk_helpers() {
        let m = Manifest::new(vec![
            f("a", T0, vec![c(3, 10), c(1, 20), c(3, 10)]),
            f("b", T0, vec![c(2, 5), c(1, 20)]),
            ManifestEntry::dir("d", T0),
        ])
        .unwrap();
        let mut expected = vec![
            ChunkHash::of(&[1]),
            ChunkHash::of(&[2]),
            ChunkHash::of(&[3]),
        ];
        expected.sort();
        assert_eq!(m.chunk_hashes().into_iter().collect::<Vec<_>>(), expected);

        let available: HashSet<ChunkHash> = [ChunkHash::of(&[1])].into();
        let mut missing = vec![ChunkHash::of(&[2]), ChunkHash::of(&[3])];
        missing.sort();
        assert_eq!(m.missing_chunks(|h| available.contains(h)), missing);
        assert!(m.missing_chunks(|_| true).is_empty());

        assert_eq!(m.logical_size(), 40 + 25);
        assert_eq!(m.unique_chunk_size(), 10 + 20 + 5);
    }
}
