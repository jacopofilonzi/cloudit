//! Manifest validation. Every manifest is validated before it is encoded and after it is
//! decoded, so the server never stores, and a client never applies, one that breaks these
//! rules:
//!
//! 1. `version` is [`MANIFEST_VERSION`].
//! 2. Every path passes [`validate_path`].
//! 3. Entries are strictly sorted by path (UTF-8 byte order), hence no duplicates.
//! 4. No two paths, including the implied parent directories of every entry, are equal
//!    after Unicode lowercasing (`str::to_lowercase`) unless they are byte-identical:
//!    `Save.dat` and `save.dat`, or `A/x` and `a/y`, would collide on Windows/macOS. This is
//!    an approximation of the file systems' case folding, deliberately stricter in places
//!    (it may reject a pair a given file system would accept), never looser for ASCII.
//! 5. No entry lives inside a path that is itself an entry: a file cannot be a parent
//!    (`a` file + `a/b`), and a `dir` entry must be empty (`a` dir + `a/b`).
//! 6. A `dir` has size 0, no chunks and `executable = false`.
//! 7. A file's chunk sizes are each in `1..=MAX_CHUNK_SIZE` and sum to its `size`.
//!    The minimum chunk size is not enforced (the validator does not depend on the
//!    chunker's exact output).
//! 8. `mtime.nanos < 1_000_000_000`.
//! 9. A chunk hash has the same size everywhere it appears (same content, same length).

use std::collections::HashMap;

use super::model::{EntryKind, MANIFEST_VERSION, Manifest, ManifestEntry};
use crate::chunk::MAX_CHUNK_SIZE;
use crate::error::ManifestError;
use crate::hash::ChunkHash;
use crate::path::validate_path;

impl Manifest {
    /// Checks every rule listed in [`crate::manifest::validate`].
    ///
    /// # Errors
    ///
    /// The first broken rule, as a [`ManifestError`].
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.version != MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion(u64::from(self.version)));
        }
        for entry in &self.entries {
            validate_entry(entry)?;
        }
        for pair in self.entries.windows(2) {
            if let [previous, entry] = pair {
                match previous.path.cmp(&entry.path) {
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Equal => {
                        return Err(ManifestError::Duplicate(entry.path.clone()));
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(ManifestError::NotSorted {
                            previous: previous.path.clone(),
                            path: entry.path.clone(),
                        });
                    }
                }
            }
        }
        validate_tree(&self.entries)?;
        validate_chunk_sizes(&self.entries)
    }
}

/// Rule 9.
fn validate_chunk_sizes(entries: &[ManifestEntry]) -> Result<(), ManifestError> {
    let mut sizes: HashMap<ChunkHash, u32> = HashMap::new();
    for chunk in entries.iter().flat_map(|e| &e.chunks) {
        if *sizes.entry(chunk.hash).or_insert(chunk.size) != chunk.size {
            return Err(ManifestError::ChunkSizeConflict { hash: chunk.hash });
        }
    }
    Ok(())
}

fn validate_entry(entry: &ManifestEntry) -> Result<(), ManifestError> {
    validate_path(&entry.path).map_err(|source| ManifestError::InvalidPath {
        path: entry.path.clone(),
        source,
    })?;
    if entry.mtime.nanos >= 1_000_000_000 {
        return Err(ManifestError::InvalidMtime {
            path: entry.path.clone(),
        });
    }
    match entry.kind {
        EntryKind::Dir => {
            if entry.size != 0 || !entry.chunks.is_empty() || entry.executable {
                return Err(ManifestError::DirWithContent(entry.path.clone()));
            }
        }
        EntryKind::File => {
            let mut total: u64 = 0;
            for chunk in &entry.chunks {
                if chunk.size == 0 || chunk.size > MAX_CHUNK_SIZE {
                    return Err(ManifestError::ChunkSizeOutOfRange {
                        path: entry.path.clone(),
                        size: chunk.size,
                    });
                }
                // Cannot overflow in practice (u32 sizes), but saturate rather than wrap.
                total = total.saturating_add(u64::from(chunk.size));
            }
            if total != entry.size {
                return Err(ManifestError::SizeMismatch {
                    path: entry.path.clone(),
                    size: entry.size,
                    chunks_total: total,
                });
            }
        }
    }
    Ok(())
}

/// Rules 4 and 5: case collisions and entries nested under files or empty dirs.
fn validate_tree(entries: &[ManifestEntry]) -> Result<(), ManifestError> {
    let kinds: HashMap<&str, EntryKind> =
        entries.iter().map(|e| (e.path.as_str(), e.kind)).collect();
    // Lowercased path -> the original spelling first seen for it.
    let mut folded: HashMap<String, &str> = HashMap::new();
    for entry in entries {
        let path = entry.path.as_str();
        for (i, _) in path.match_indices('/') {
            let parent = &path[..i];
            match kinds.get(parent) {
                Some(EntryKind::File) => {
                    return Err(ManifestError::ParentIsFile {
                        parent: parent.to_owned(),
                        child: path.to_owned(),
                    });
                }
                Some(EntryKind::Dir) => {
                    return Err(ManifestError::DirHasChildren {
                        parent: parent.to_owned(),
                        child: path.to_owned(),
                    });
                }
                None => {}
            }
            check_case(&mut folded, parent)?;
        }
        check_case(&mut folded, path)?;
    }
    Ok(())
}

fn check_case<'a>(
    folded: &mut HashMap<String, &'a str>,
    path: &'a str,
) -> Result<(), ManifestError> {
    let key = path.to_lowercase();
    match folded.get(&key) {
        Some(&seen) if seen != path => Err(ManifestError::CaseCollision {
            a: seen.to_owned(),
            b: path.to_owned(),
        }),
        Some(_) => Ok(()),
        None => {
            folded.insert(key, path);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PathError;
    use crate::hash::ChunkHash;
    use crate::manifest::model::{ChunkRef, Timestamp};

    const T: Timestamp = Timestamp {
        secs: 1_700_000_000,
        nanos: 0,
    };

    fn chunk(n: u8, size: u32) -> ChunkRef {
        ChunkRef {
            hash: ChunkHash::of(&[n]),
            size,
        }
    }

    fn file(path: &str) -> ManifestEntry {
        ManifestEntry::file(path, T, false, vec![chunk(1, 10)])
    }

    fn raw(entries: Vec<ManifestEntry>) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            entries,
        }
    }

    #[test]
    fn valid_manifest_passes_and_new_sorts() {
        let m = Manifest::new(vec![
            file("b/c.txt"),
            ManifestEntry::dir("empty", T),
            file("a.txt"),
            file("a/x"),
            ManifestEntry::file("zero", T, true, vec![]),
            file("b/C.txt.bak"),
        ])
        .unwrap();
        let paths: Vec<&str> = m.entries.iter().map(|e| e.path.as_str()).collect();
        // '.' (0x2E) sorts before '/' (0x2F): byte order, not "directory first".
        assert_eq!(
            paths,
            ["a.txt", "a/x", "b/C.txt.bak", "b/c.txt", "empty", "zero"]
        );
        assert!(m.get("b/c.txt").is_some());
        assert!(m.get("b").is_none());
        assert!(raw(vec![]).validate().is_ok());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut m = raw(vec![]);
        m.version = 2;
        assert!(matches!(
            m.validate(),
            Err(ManifestError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn rejects_invalid_path() {
        let err = raw(vec![file("../escape")]).validate().unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidPath { ref path, source: PathError::DotSegment } if path == "../escape"
        ));
        assert_eq!(err.code(), "manifest_invalid_path");
    }

    #[test]
    fn rejects_unsorted() {
        let err = raw(vec![file("b"), file("a")]).validate().unwrap_err();
        assert!(matches!(err, ManifestError::NotSorted { .. }));
    }

    #[test]
    fn rejects_duplicate() {
        let err = raw(vec![file("a"), file("a")]).validate().unwrap_err();
        assert!(matches!(err, ManifestError::Duplicate(ref p) if p == "a"));
        let err = Manifest::new(vec![file("a"), ManifestEntry::dir("a", T)]).unwrap_err();
        assert!(matches!(err, ManifestError::Duplicate(_)));
    }

    #[test]
    fn rejects_case_collision() {
        for (a, b) in [
            ("Save.dat", "save.dat"),
            ("A/x", "a/y"),
            ("dir/File", "Dir/other"),
            ("ÀB", "àb"),
            ("STRAßE", "straße"),
            ("ΣΊΣΥΦΟΣ", "σίσυφος"),
        ] {
            let err = Manifest::new(vec![file(a), file(b)]).unwrap_err();
            assert!(
                matches!(err, ManifestError::CaseCollision { .. }),
                "{a} / {b}: {err:?}"
            );
        }
        // A file `A` and an entry inside `a/`: collision via the implied parent.
        let err = Manifest::new(vec![file("A"), file("a/b")]).unwrap_err();
        assert!(
            matches!(err, ManifestError::CaseCollision { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_file_used_as_parent() {
        let err = Manifest::new(vec![file("a"), file("a/b")]).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::ParentIsFile { ref parent, ref child } if parent == "a" && child == "a/b"
        ));
        let err = Manifest::new(vec![file("a/b"), file("a/b/c/d")]).unwrap_err();
        assert!(matches!(err, ManifestError::ParentIsFile { .. }));
    }

    #[test]
    fn rejects_non_empty_dir_entry() {
        let err = Manifest::new(vec![ManifestEntry::dir("d", T), file("d/x")]).unwrap_err();
        assert!(matches!(err, ManifestError::DirHasChildren { .. }));
    }

    #[test]
    fn rejects_dir_with_content() {
        let mut sized = ManifestEntry::dir("d", T);
        sized.size = 1;
        let mut chunked = ManifestEntry::dir("d", T);
        chunked.chunks.push(chunk(1, 1));
        let mut exec = ManifestEntry::dir("d", T);
        exec.executable = true;
        for entry in [sized, chunked, exec] {
            let err = raw(vec![entry]).validate().unwrap_err();
            assert!(matches!(err, ManifestError::DirWithContent(_)));
        }
    }

    #[test]
    fn rejects_size_mismatch() {
        let mut entry = file("f");
        entry.size = 11;
        let err = raw(vec![entry]).validate().unwrap_err();
        assert!(matches!(
            err,
            ManifestError::SizeMismatch {
                size: 11,
                chunks_total: 10,
                ..
            }
        ));
        let mut empty_with_size = ManifestEntry::file("f", T, false, vec![]);
        empty_with_size.size = 5;
        assert!(matches!(
            raw(vec![empty_with_size]).validate(),
            Err(ManifestError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn rejects_chunk_size_out_of_range() {
        for size in [0, MAX_CHUNK_SIZE + 1] {
            let entry = ManifestEntry::file("f", T, false, vec![chunk(1, size)]);
            let err = raw(vec![entry]).validate().unwrap_err();
            assert!(
                matches!(err, ManifestError::ChunkSizeOutOfRange { .. }),
                "{size}"
            );
        }
        let ok = ManifestEntry::file("f", T, false, vec![chunk(1, MAX_CHUNK_SIZE), chunk(2, 1)]);
        assert!(raw(vec![ok]).validate().is_ok());
    }

    #[test]
    fn rejects_chunk_size_conflict() {
        let a = ManifestEntry::file("a", T, false, vec![chunk(1, 10)]);
        let b = ManifestEntry::file("b", T, false, vec![chunk(1, 11)]);
        let err = Manifest::new(vec![a.clone(), b]).unwrap_err();
        assert!(matches!(err, ManifestError::ChunkSizeConflict { .. }));
        let same = ManifestEntry::file("b", T, false, vec![chunk(1, 10), chunk(1, 10)]);
        assert!(Manifest::new(vec![a, same]).is_ok());
    }

    #[test]
    fn rejects_invalid_mtime() {
        let mut entry = file("f");
        entry.mtime.nanos = 1_000_000_000;
        assert!(matches!(
            raw(vec![entry]).validate(),
            Err(ManifestError::InvalidMtime { .. })
        ));
        let mut entry = file("f");
        entry.mtime = Timestamp {
            secs: -1,
            nanos: 999_999_999,
        };
        assert!(raw(vec![entry]).validate().is_ok());
    }
}
