//! Pure planning: the three-way comparison behind the folder status, the pull plan and
//! the safety check (table-tested; no I/O).
//!
//! Every comparison is by path over the manifests with ignored paths removed, on content
//! only ([`same`]: kind, size, chunks, executable; the mtime never counts). For each
//! path, with `L` local, `B` base (last synced), `H` server head:
//!
//! - `L == H`: converged (nothing to do, whatever `B` is);
//! - else `L != B` and `H == B`: a local change;
//! - else `H != B` and `L == B`: a remote change;
//! - else both changed it: a conflicting path.
//!
//! Because converged paths never count, a pull interrupted halfway resumes as a plain
//! pull (the files already replaced equal `H`), and two devices making the same change
//! don't conflict.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, EntryKind, Manifest, ManifestEntry};
use serde::Serialize;

use crate::scan::{FileState, IgnoreRules};

/// Same content (mtime ignored); `None` = absent.
#[must_use]
pub fn same(a: Option<&ManifestEntry>, b: Option<&ManifestEntry>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_content(b),
        _ => false,
    }
}

/// The entries of `manifest` that are not ignored, by path.
#[must_use]
pub fn visible<'a>(
    manifest: Option<&'a Manifest>,
    rules: &IgnoreRules,
) -> BTreeMap<&'a str, &'a ManifestEntry> {
    manifest
        .map(|m| {
            m.entries
                .iter()
                .filter(|e| !rules.is_ignored(&e.path, e.kind == EntryKind::Dir))
                .map(|e| (e.path.as_str(), e))
                .collect()
        })
        .unwrap_or_default()
}

/// How a path changed on one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename = "PathChangeKind")
)]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PathChange {
    pub path: String,
    pub change: ChangeKind,
}

fn change_of(before: Option<&ManifestEntry>, after: Option<&ManifestEntry>) -> ChangeKind {
    match (before, after) {
        (None, _) => ChangeKind::Added,
        (_, None) => ChangeKind::Removed,
        _ => ChangeKind::Modified,
    }
}

/// The three-way comparison of a folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Comparison {
    /// Paths only this device changed (relative to the base).
    pub local: Vec<PathChange>,
    /// Paths only the server changed.
    pub remote: Vec<PathChange>,
    /// Paths both changed, differently.
    pub conflicting: Vec<String>,
}

impl Comparison {
    /// Nothing to do: the local content equals the head's.
    #[must_use]
    pub fn converged(&self) -> bool {
        self.local.is_empty() && self.remote.is_empty() && self.conflicting.is_empty()
    }

    /// Both sides changed something: the folder needs a decision.
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        !self.conflicting.is_empty() || (!self.local.is_empty() && !self.remote.is_empty())
    }
}

/// Compares the local manifest with the base and the head (see the module docs).
#[must_use]
pub fn compare(
    local: &Manifest,
    base: Option<&Manifest>,
    head: Option<&Manifest>,
    rules: &IgnoreRules,
) -> Comparison {
    let l = visible(Some(local), rules);
    let b = visible(base, rules);
    let h = visible(head, rules);
    let paths: BTreeSet<&str> = l.keys().chain(b.keys()).chain(h.keys()).copied().collect();
    let mut out = Comparison::default();
    for path in paths {
        let (le, be, he) = (
            l.get(path).copied(),
            b.get(path).copied(),
            h.get(path).copied(),
        );
        if same(le, he) {
            continue;
        }
        match (same(le, be), same(he, be)) {
            (false, true) => out.local.push(PathChange {
                path: path.to_owned(),
                change: change_of(be, le),
            }),
            (true, false) => out.remote.push(PathChange {
                path: path.to_owned(),
                change: change_of(be, he),
            }),
            _ => out.conflicting.push(path.to_owned()),
        }
    }
    out
}

/// The manifest to upload: the local entries plus the parent's entries under paths this
/// device ignores (ignoring never deletes them for other devices).
pub fn upload_manifest(
    local: &Manifest,
    parent: Option<&Manifest>,
    rules: &IgnoreRules,
) -> Result<Manifest, cloudit_core::error::ManifestError> {
    let mut entries = local.entries.clone();
    if let Some(parent) = parent {
        let present: HashSet<&str> = local.entries.iter().map(|e| e.path.as_str()).collect();
        entries.extend(
            parent
                .entries
                .iter()
                .filter(|e| {
                    rules.is_ignored(&e.path, e.kind == EntryKind::Dir)
                        && !present.contains(e.path.as_str())
                })
                .cloned(),
        );
    }
    Manifest::new(entries)
}

/// What a local path holds before a pull touches it (from the scan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// Nothing (or only an implicit directory).
    Absent,
    /// This file (re-checked by size + mtime, and by hash when racy).
    File {
        state: FileState,
        chunks: Vec<ChunkRef>,
    },
    /// An empty directory.
    EmptyDir,
}

/// A file the pull writes.
#[derive(Debug, Clone)]
pub struct Write {
    pub entry: ManifestEntry,
    pub expect: Expect,
}

/// A local item the pull removes.
#[derive(Debug, Clone)]
pub struct Remove {
    pub path: String,
    pub expect: Expect,
}

/// The changes that make the local folder equal the head (ignored paths untouched).
#[derive(Debug, Clone, Default)]
pub struct PullPlan {
    /// Files to write (new or replaced), by path.
    pub writes: Vec<Write>,
    /// Empty directories to create.
    pub mkdirs: Vec<String>,
    /// Files and empty directories to remove.
    pub removes: Vec<Remove>,
}

impl PullPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty() && self.mkdirs.is_empty() && self.removes.is_empty()
    }

    /// Number of file-system changes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.writes.len() + self.mkdirs.len() + self.removes.len()
    }

    /// The local files this plan overwrites or deletes, with their current chunks.
    pub fn replaced_files(&self) -> impl Iterator<Item = (&str, &[ChunkRef])> {
        let writes = self.writes.iter().filter_map(|w| match &w.expect {
            Expect::File { chunks, .. } => Some((w.entry.path.as_str(), chunks.as_slice())),
            _ => None,
        });
        let removes = self.removes.iter().filter_map(|r| match &r.expect {
            Expect::File { chunks, .. } => Some((r.path.as_str(), chunks.as_slice())),
            _ => None,
        });
        writes.chain(removes)
    }
}

fn expect_of(entry: Option<&ManifestEntry>, files: &HashMap<String, FileState>) -> Expect {
    match entry {
        None => Expect::Absent,
        Some(e) if e.kind == EntryKind::Dir => Expect::EmptyDir,
        Some(e) => Expect::File {
            state: files.get(&e.path).copied().unwrap_or(FileState {
                size: e.size,
                mtime: e.mtime,
                racy: true,
            }),
            chunks: e.chunks.clone(),
        },
    }
}

/// Plans the pull of `head` over the scanned `local` state.
#[must_use]
pub fn plan_pull(
    local: &Manifest,
    files: &HashMap<String, FileState>,
    head: &Manifest,
    rules: &IgnoreRules,
) -> PullPlan {
    let l = visible(Some(local), rules);
    let h = visible(Some(head), rules);
    let paths: BTreeSet<&str> = l.keys().chain(h.keys()).copied().collect();
    let mut plan = PullPlan::default();
    for path in paths {
        let (le, he) = (l.get(path).copied(), h.get(path).copied());
        if same(le, he) {
            continue;
        }
        let expect = expect_of(le, files);
        match he {
            Some(target) if target.kind == EntryKind::File => plan.writes.push(Write {
                entry: target.clone(),
                expect,
            }),
            Some(_) => {
                if matches!(expect, Expect::File { .. }) {
                    plan.removes.push(Remove {
                        path: path.to_owned(),
                        expect,
                    });
                }
                plan.mkdirs.push(path.to_owned());
            }
            None => plan.removes.push(Remove {
                path: path.to_owned(),
                expect,
            }),
        }
    }
    plan
}

/// Content known to be stored on the server: the chunk lists of files in snapshots
/// that exist there (base if still there, head, this device's pending conflict).
#[derive(Debug, Clone, Default)]
pub struct SafeSet {
    contents: HashSet<Vec<ChunkRef>>,
}

impl SafeSet {
    /// Adds every file content of `manifest`.
    pub fn add(&mut self, manifest: &Manifest) {
        for entry in &manifest.entries {
            if entry.kind == EntryKind::File {
                self.contents.insert(entry.chunks.clone());
            }
        }
    }

    /// Whether a file with these chunks is stored on the server (an empty file always
    /// is: there is nothing to lose).
    #[must_use]
    pub fn contains(&self, chunks: &[ChunkRef]) -> bool {
        chunks.is_empty() || self.contents.contains(chunks)
    }
}

/// The paths `plan` would overwrite or delete whose current content is not in `safe`.
#[must_use]
pub fn unsafe_paths(plan: &PullPlan, safe: &SafeSet) -> Vec<String> {
    plan.replaced_files()
        .filter(|(_, chunks)| !safe.contains(chunks))
        .map(|(path, _)| path.to_owned())
        .collect()
}

/// Every chunk the plan's writes need, with its size.
#[must_use]
pub fn needed_chunks(plan: &PullPlan) -> BTreeMap<ChunkHash, u32> {
    plan.writes
        .iter()
        .flat_map(|w| w.entry.chunks.iter())
        .map(|c| (c.hash, c.size))
        .collect()
}

#[cfg(test)]
mod tests {
    use cloudit_core::manifest::Timestamp;

    use super::*;

    const T0: Timestamp = Timestamp { secs: 1, nanos: 0 };
    const T1: Timestamp = Timestamp { secs: 2, nanos: 0 };

    fn file(path: &str, content: &str, mtime: Timestamp) -> ManifestEntry {
        let chunks = if content.is_empty() {
            Vec::new()
        } else {
            vec![ChunkRef {
                hash: ChunkHash::of(content.as_bytes()),
                size: u32::try_from(content.len()).unwrap(),
            }]
        };
        ManifestEntry::file(path, mtime, false, chunks)
    }

    fn m(entries: &[ManifestEntry]) -> Manifest {
        Manifest::new(entries.to_vec()).unwrap()
    }

    fn paths(changes: &[PathChange]) -> Vec<(&str, ChangeKind)> {
        changes
            .iter()
            .map(|c| (c.path.as_str(), c.change))
            .collect()
    }

    #[test]
    fn three_way_classification() {
        let base = m(&[
            file("a", "1", T0),
            file("b", "1", T0),
            file("c", "1", T0),
            file("d", "1", T0),
        ]);
        // a: local edit; b: remote edit; c: both, differently; d: both, the same way;
        // e: local add; f: remote add; mtime-only change of b locally is ignored.
        let local = m(&[
            file("a", "2", T0),
            file("b", "1", T1),
            file("c", "L", T0),
            file("d", "X", T0),
            file("e", "1", T0),
        ]);
        let head = m(&[
            file("a", "1", T0),
            file("b", "2", T0),
            file("c", "H", T0),
            file("d", "X", T1),
            file("f", "1", T0),
        ]);
        let rules = IgnoreRules::default();
        let cmp = compare(&local, Some(&base), Some(&head), &rules);
        assert_eq!(
            paths(&cmp.local),
            vec![("a", ChangeKind::Modified), ("e", ChangeKind::Added)]
        );
        assert_eq!(
            paths(&cmp.remote),
            vec![("b", ChangeKind::Modified), ("f", ChangeKind::Added)]
        );
        assert_eq!(cmp.conflicting, vec!["c".to_owned()]);
        assert!(cmp.is_conflict());

        let synced = compare(&base, Some(&base), Some(&base), &rules);
        assert!(synced.converged());
        let deleted = compare(&m(&[]), Some(&base), Some(&base), &rules);
        assert_eq!(deleted.local.len(), 4);
        assert!(
            deleted
                .local
                .iter()
                .all(|c| c.change == ChangeKind::Removed)
        );
        // No base: everything both sides have, differently, conflicts.
        let first = compare(
            &m(&[file("a", "1", T0)]),
            None,
            Some(&m(&[file("a", "2", T0)])),
            &rules,
        );
        assert_eq!(first.conflicting, vec!["a".to_owned()]);
        let same_first = compare(
            &m(&[file("a", "1", T0)]),
            None,
            Some(&m(&[file("a", "1", T1)])),
            &rules,
        );
        assert!(same_first.converged());
    }

    #[test]
    fn ignored_paths_never_count_and_are_carried_over() {
        let rules = IgnoreRules::new(&["*.log".to_owned()]).unwrap();
        let head = m(&[file("a", "1", T0), file("x.log", "server", T0)]);
        let local = m(&[file("a", "1", T0)]);
        assert!(compare(&local, Some(&head), Some(&head), &rules).converged());
        let upload = upload_manifest(&m(&[file("a", "2", T0)]), Some(&head), &rules).unwrap();
        assert_eq!(upload.entries.len(), 2);
        assert!(upload.get("x.log").is_some());
        let plan = plan_pull(&local, &HashMap::new(), &head, &rules);
        assert!(plan.is_empty());
    }

    #[test]
    fn pull_plan_and_safety() {
        let local = m(&[
            file("keep", "1", T0),
            file("edit", "old", T0),
            file("gone", "lost", T0),
            file("becomes-dir", "f", T0),
            ManifestEntry::dir("old-empty", T0),
        ]);
        let head = m(&[
            file("keep", "1", T1),
            file("edit", "new", T0),
            file("new", "n", T0),
            ManifestEntry::dir("becomes-dir", T0),
        ]);
        let rules = IgnoreRules::default();
        let plan = plan_pull(&local, &HashMap::new(), &head, &rules);
        let writes: Vec<_> = plan.writes.iter().map(|w| w.entry.path.as_str()).collect();
        assert_eq!(writes, vec!["edit", "new"]);
        assert!(matches!(plan.writes[0].expect, Expect::File { .. }));
        assert_eq!(plan.writes[1].expect, Expect::Absent);
        assert_eq!(plan.mkdirs, vec!["becomes-dir".to_owned()]);
        let removes: Vec<_> = plan.removes.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(removes, vec!["becomes-dir", "gone", "old-empty"]);

        // Nothing local is stored: three files would be lost.
        let mut safe = SafeSet::default();
        safe.add(&head);
        assert_eq!(
            unsafe_paths(&plan, &safe),
            vec!["edit", "becomes-dir", "gone"]
        );
        // With the local state stored (e.g. as a conflict snapshot), all is safe.
        safe.add(&local);
        assert!(unsafe_paths(&plan, &safe).is_empty());
        assert_eq!(needed_chunks(&plan).len(), 2);
    }
}
