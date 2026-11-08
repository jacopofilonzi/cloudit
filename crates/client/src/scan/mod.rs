//! Folder scanner: walks a linked folder and builds its [`Manifest`].
//!
//! - `walkdir` without following symlinks; symlinks and special files (sockets, FIFOs,
//!   devices) are skipped (logged), ignored paths ([`IgnoreRules`]) are never entered.
//! - Relative paths use `/`; each one is checked with `cloudit_core::path::validate_path`
//!   and against case-insensitive collisions (`A/x` vs `a/y`). Bad ones, non-UTF-8 names
//!   and unreadable entries (a game holding a save locked, missing permissions) are
//!   reported as [`ScanProblem`]s, one per item, and left out of the manifest: callers
//!   refuse to upload or apply anything while there are problems (a partial manifest
//!   would delete the missing files on the server).
//! - Files whose size and mtime match the file index reuse its chunk list; others are
//!   read with the streaming chunker (`chunk_reader`, one 1 MiB buffer). A file whose
//!   size or mtime changes while it is read is read again (twice at most). Files
//!   modified within [`RACY_WINDOW`] of the scan are "racy": a same-size rewrite in the
//!   same mtime tick couldn't be told apart, so they are not cached in the index and
//!   the apply re-check hashes them.
//! - Directories with no synced content become `dir` entries (empty directories).
//! - Where files have no executable bit (Windows), a file keeps the flag of the base
//!   manifest's entry at the same path.

pub mod ignore;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{File, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime};

use cloudit_core::chunk::chunker::chunk_reader;
use cloudit_core::manifest::{ChunkRef, EntryKind, Manifest, ManifestEntry, Timestamp};
use cloudit_core::path::validate_path;
pub use ignore::IgnoreRules;
use serde::Serialize;
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::platform;
use crate::progress::CancellationToken;
use crate::state::index::IndexEntry;
use crate::sync::stage::TEMP_PREFIX;

/// Files modified this close to the scan start are re-hashed when it matters.
pub const RACY_WINDOW: Duration = Duration::from_secs(2);
/// Leftover temp files older than this are removed by the scanner.
const STALE_TEMP: Duration = Duration::from_secs(3600);

/// Why an item can't be synced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ProblemKind {
    /// Can't be opened or read (locked by a running program, permissions…).
    Unreadable,
    /// The name breaks the portable path rules (`cloudit_core::path`).
    InvalidName,
    /// Differs from another path only by letter case.
    NameCollision,
    /// The name is not valid Unicode.
    NotUtf8,
}

/// An item that can't be synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ScanProblem {
    /// Relative path (lossy for non-UTF-8 names).
    pub path: String,
    pub kind: ProblemKind,
    /// English detail (e.g. the OS error, the rule broken, the other path).
    pub detail: String,
}

/// What the scan saw of one file, for the re-check before overwriting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileState {
    pub size: u64,
    pub mtime: Timestamp,
    /// Modified within [`RACY_WINDOW`] of the scan: the re-check also compares hashes.
    pub racy: bool,
}

/// A scanned folder.
#[derive(Debug, Clone)]
pub struct Scan {
    /// The synced content (problem items left out).
    pub manifest: Manifest,
    /// Size and mtime of every file in `manifest`.
    pub files: HashMap<String, FileState>,
    /// Items that can't be synced.
    pub problems: Vec<ScanProblem>,
    /// Non-racy files, to store in the index.
    pub index: Vec<(String, IndexEntry)>,
    /// Bytes read to hash changed files.
    pub hashed_bytes: u64,
}

impl Scan {
    /// `Err(ScanProblems)` unless the scan is complete.
    pub fn require_complete(&self) -> Result<()> {
        if self.problems.is_empty() {
            Ok(())
        } else {
            Err(Error::ScanProblems(self.problems.clone()))
        }
    }
}

/// Inputs of [`scan`].
pub struct ScanInput<'a> {
    pub root: &'a Path,
    pub rules: &'a IgnoreRules,
    pub index: &'a HashMap<String, IndexEntry>,
    /// Supplies the executable flag where the OS has none.
    pub base: Option<&'a Manifest>,
    pub cancel: &'a CancellationToken,
    /// Called with (files seen, bytes hashed) now and then.
    pub progress: &'a (dyn Fn(u64, u64) + Sync),
}

struct Walked {
    files: BTreeMap<String, (Metadata, std::path::PathBuf)>,
    dirs: BTreeSet<String>,
    problems: Vec<ScanProblem>,
}

/// Scans a folder (blocking: run it on a blocking thread).
pub fn scan(input: &ScanInput<'_>) -> Result<Scan> {
    let started = SystemTime::now();
    let racy_after = platform::timestamp(started.checked_sub(RACY_WINDOW).unwrap_or(started));
    let walked = walk(input, started)?;
    let mut problems = walked.problems;

    // Names: portable rules, then case collisions among everything left.
    let mut names_ok: BTreeSet<String> = BTreeSet::new();
    for path in walked.files.keys().chain(walked.dirs.iter()) {
        match validate_path(path) {
            Ok(()) => {
                names_ok.insert(path.clone());
            }
            Err(e) => problems.push(ScanProblem {
                path: path.clone(),
                kind: ProblemKind::InvalidName,
                detail: format!("{} ({e})", e.code()),
            }),
        }
    }
    let colliding = collisions(&names_ok);
    for (path, other) in &colliding {
        problems.push(ScanProblem {
            path: path.clone(),
            kind: ProblemKind::NameCollision,
            detail: format!("differs only by letter case from {other:?}"),
        });
    }
    let blocked = |path: &str| {
        !names_ok.contains(path)
            || colliding
                .keys()
                .any(|bad| path == bad || path.starts_with(&format!("{bad}/")))
    };

    let mut entries = Vec::new();
    let mut files = HashMap::new();
    let mut index = Vec::new();
    let mut hashed_bytes = 0u64;
    let mut seen = 0u64;
    for (path, (meta, full)) in &walked.files {
        if input.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        seen += 1;
        if seen.is_multiple_of(256) {
            (input.progress)(seen, hashed_bytes);
        }
        if blocked(path) {
            continue;
        }
        let executable = if platform::supports_exec_bit() {
            platform::is_executable(meta)
        } else {
            input
                .base
                .and_then(|b| b.get(path))
                .is_some_and(|e| e.kind == EntryKind::File && e.executable)
        };
        match file_chunks(input, path, full, meta, &mut hashed_bytes) {
            Ok((state, chunks)) => {
                let racy = state.mtime >= racy_after;
                let state = FileState { racy, ..state };
                if !racy {
                    index.push((
                        path.clone(),
                        IndexEntry {
                            size: state.size,
                            mtime: state.mtime,
                            executable,
                            chunks: chunks.clone(),
                        },
                    ));
                }
                let mut entry = ManifestEntry::file(path.clone(), state.mtime, executable, chunks);
                entry.size = state.size;
                entries.push(entry);
                files.insert(path.clone(), state);
            }
            Err(FileError::Cancelled) => return Err(Error::Cancelled),
            Err(FileError::Problem(detail)) => problems.push(ScanProblem {
                path: path.clone(),
                kind: ProblemKind::Unreadable,
                detail,
            }),
        }
    }
    (input.progress)(seen, hashed_bytes);

    // Empty directories: no synced entry below them (problem items count as content,
    // so a directory holding only a locked file isn't recorded as empty).
    let mut has_content: HashSet<&str> = HashSet::new();
    for path in walked.files.keys().chain(walked.dirs.iter()) {
        let mut rest = path.as_str();
        while let Some((parent, _)) = rest.rsplit_once('/') {
            if !has_content.insert(parent) {
                break;
            }
            rest = parent;
        }
    }
    for dir in &walked.dirs {
        if !has_content.contains(dir.as_str()) && !blocked(dir) {
            let mtime = std::fs::metadata(input.root.join(dir))
                .and_then(|m| platform::mtime(&m))
                .unwrap_or(Timestamp { secs: 0, nanos: 0 });
            entries.push(ManifestEntry::dir(dir.clone(), mtime));
        }
    }

    let manifest = Manifest::new(entries).map_err(|e| {
        Error::ScanProblems(vec![ScanProblem {
            path: String::new(),
            kind: ProblemKind::InvalidName,
            detail: format!("{} ({e})", e.code()),
        }])
    })?;
    problems.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Scan {
        manifest,
        files,
        problems,
        index,
        hashed_bytes,
    })
}

/// Walks the tree: files with their metadata, directories, problems.
fn walk(input: &ScanInput<'_>, started: SystemTime) -> Result<Walked> {
    let root_meta = std::fs::metadata(input.root)
        .map_err(|e| Error::io(format!("reading {}", input.root.display()), e))?;
    if !root_meta.is_dir() {
        return Err(Error::io(
            format!("{} is not a directory", input.root.display()),
            std::io::Error::from(std::io::ErrorKind::NotADirectory),
        ));
    }
    let mut out = Walked {
        files: BTreeMap::new(),
        dirs: BTreeSet::new(),
        problems: Vec::new(),
    };
    let walker = WalkDir::new(input.root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| match relative(input.root, entry.path()) {
            Some(Ok(rel)) => !input.rules.matches(&rel, entry.file_type().is_dir()),
            // Reported below.
            _ => true,
        });
    for item in walker {
        if input.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let entry = match item {
            Ok(entry) => entry,
            Err(e) => {
                let path = e
                    .path()
                    .and_then(|p| relative(input.root, p))
                    .map(|r| r.unwrap_or_else(|lossy| lossy))
                    .unwrap_or_default();
                out.problems.push(ScanProblem {
                    path,
                    kind: ProblemKind::Unreadable,
                    detail: e.to_string(),
                });
                continue;
            }
        };
        let rel = match relative(input.root, entry.path()) {
            Some(Ok(rel)) => rel,
            Some(Err(lossy)) => {
                out.problems.push(ScanProblem {
                    path: lossy,
                    kind: ProblemKind::NotUtf8,
                    detail: "the name is not valid Unicode".to_owned(),
                });
                continue;
            }
            None => continue,
        };
        let file_type = entry.file_type();
        if file_type.is_dir() {
            out.dirs.insert(rel);
        } else if file_type.is_file() {
            let name = rel.rsplit('/').next().unwrap_or(&rel);
            if name.starts_with(TEMP_PREFIX) {
                remove_stale_temp(entry.path(), started);
                continue;
            }
            match entry.metadata() {
                Ok(meta) => {
                    out.files.insert(rel, (meta, entry.path().to_path_buf()));
                }
                Err(e) => out.problems.push(ScanProblem {
                    path: rel,
                    kind: ProblemKind::Unreadable,
                    detail: e.to_string(),
                }),
            }
        } else {
            tracing::debug!(path = %rel, "skipping a symlink or special file");
        }
    }
    Ok(out)
}

/// `root`-relative path with `/`: `Ok` if UTF-8, `Err(lossy)` if not, `None` for the
/// root itself.
fn relative(root: &Path, path: &Path) -> Option<std::result::Result<String, String>> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut lossy = false;
    for part in rel.components() {
        let os = part.as_os_str();
        match os.to_str() {
            Some(s) => parts.push(s.to_owned()),
            None => {
                lossy = true;
                parts.push(os.to_string_lossy().into_owned());
            }
        }
    }
    let joined = parts.join("/");
    Some(if lossy { Err(joined) } else { Ok(joined) })
}

/// Paths (or parents of paths) that collide with another ignoring case: path → other.
fn collisions(paths: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut out = BTreeMap::new();
    for path in paths {
        let mut end = 0;
        loop {
            let next = path.get(end..).and_then(|rest| rest.find('/'));
            let prefix = match next {
                Some(pos) => path.get(..end + pos).unwrap_or(path),
                None => path.as_str(),
            };
            let key = prefix.to_lowercase();
            match seen.get(&key) {
                Some(other) if other != prefix => {
                    out.insert(prefix.to_owned(), other.clone());
                    out.insert(other.clone(), prefix.to_owned());
                    // Report the topmost colliding name only (`A` vs `a`, not also
                    // `A/x` vs `a/x`).
                    break;
                }
                Some(_) => {}
                None => {
                    seen.insert(key, prefix.to_owned());
                }
            }
            match next {
                Some(pos) => end += pos + 1,
                None => break,
            }
        }
    }
    out
}

enum FileError {
    Cancelled,
    Problem(String),
}

/// The chunks of one file: from the index if unchanged, else hashed (re-read while it
/// keeps changing, at most twice more).
fn file_chunks(
    input: &ScanInput<'_>,
    path: &str,
    full: &Path,
    meta: &Metadata,
    hashed_bytes: &mut u64,
) -> std::result::Result<(FileState, Vec<ChunkRef>), FileError> {
    let mtime = platform::mtime(meta).map_err(|e| FileError::Problem(e.to_string()))?;
    let state = FileState {
        size: meta.len(),
        mtime,
        racy: false,
    };
    if let Some(cached) = input.index.get(path)
        && cached.size == state.size
        && cached.mtime == state.mtime
    {
        return Ok((state, cached.chunks.clone()));
    }
    for _ in 0..3 {
        let (before, chunks) = hash_file(full, input.cancel, hashed_bytes)?;
        let after = std::fs::metadata(full)
            .and_then(|m| Ok((m.len(), platform::mtime(&m)?)))
            .map_err(|e| FileError::Problem(e.to_string()))?;
        let total: u64 = chunks.iter().map(|c| u64::from(c.size)).sum();
        if before == after && total == after.0 {
            return Ok((
                FileState {
                    size: after.0,
                    mtime: after.1,
                    racy: false,
                },
                chunks,
            ));
        }
    }
    Err(FileError::Problem(
        "the file keeps changing while it is read".to_owned(),
    ))
}

/// Hashes a file: its (size, mtime) when opened and its chunks.
fn hash_file(
    full: &Path,
    cancel: &CancellationToken,
    hashed_bytes: &mut u64,
) -> std::result::Result<((u64, Timestamp), Vec<ChunkRef>), FileError> {
    let file = File::open(full).map_err(|e| FileError::Problem(e.to_string()))?;
    let meta = file
        .metadata()
        .map_err(|e| FileError::Problem(e.to_string()))?;
    let opened = (
        meta.len(),
        platform::mtime(&meta).map_err(|e| FileError::Problem(e.to_string()))?,
    );
    let mut chunks = Vec::new();
    for chunk in chunk_reader(file) {
        if cancel.is_cancelled() {
            return Err(FileError::Cancelled);
        }
        let chunk = chunk.map_err(|e| FileError::Problem(e.0.to_string()))?;
        *hashed_bytes += u64::from(chunk.len());
        chunks.push(ChunkRef {
            hash: chunk.hash,
            size: chunk.len(),
        });
    }
    Ok((opened, chunks))
}

/// Removes an engine temp file left by a crash, once it is clearly not in use.
fn remove_stale_temp(path: &Path, now: SystemTime) {
    let old = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|modified| {
            now.duration_since(modified)
                .is_ok_and(|age| age > STALE_TEMP)
        });
    if old {
        match std::fs::remove_file(path) {
            Ok(()) => tracing::info!(path = %path.display(), "removed a leftover temp file"),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not remove a leftover temp file");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_collisions_on_parents_too() {
        let paths: BTreeSet<String> = ["A", "A/x", "a", "a/x", "a/y", "b", "c/d", "C"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let found = collisions(&paths);
        assert_eq!(found.get("A").map(String::as_str), Some("a"));
        assert_eq!(found.get("a").map(String::as_str), Some("A"));
        assert_eq!(found.get("C").map(String::as_str), Some("c"));
        assert!(!found.contains_key("b"));
        // Only the topmost name of a colliding pair is reported.
        assert_eq!(found.len(), 4, "{found:?}");
    }
}
