//! Writing pulled files safely: staging, re-checking, atomic replacement.
//!
//! **Temp files.** Every new or changed file is first written completely to a temp file
//! `.cloudit-tmp-<16 hex>` in the **root of the linked folder** (same volume as the
//! targets, so the final rename is atomic; a target directory on another volume, i.e. a
//! mount point inside the folder, gets a copy next to it first). Its content is
//! assembled from chunks already in local files (verified by hash) or downloaded to the
//! engine's chunk cache (verified by `decode_chunk`, again by hash when read), then its
//! mtime and executable bit are set and it is fsynced. Temp files are removed on any
//! failure or cancellation ([`TempFiles`] cleans up on drop); the scanner always ignores
//! the prefix and deletes leftovers older than an hour (after a crash), so they never
//! reach a manifest. They are the only thing the engine ever creates inside a synced
//! folder besides the synced files.
//!
//! **Apply.** Only once every file is staged: removals first (files, then empty
//! directories, deepest first), then directories and writes in path order. Right before
//! touching each path, [`recheck`] compares it with what the scan saw (missing, an
//! empty directory, or a file with the same size and mtime, and the same chunks when
//! the scan found it racy). Anything else means the file changed since the scan (e.g. a
//! game just saved): the apply stops there, removes the remaining temp files and
//! reports [`Error::LocalChanged`]; the file is never touched. Each replacement is one
//! rename, so every file is always either its old or its new version. The remaining
//! window between the re-check and the rename is a few system calls.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use cloudit_core::chunk::chunker::chunk_reader;
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{ChunkRef, Manifest, ManifestEntry};

use super::plan::{Expect, PullPlan};
use crate::error::{Error, Result};
use crate::platform;
use crate::progress::{OpContext, Phase};
use crate::state::index::IndexEntry;

/// Prefix of the engine's temp files inside synced folders.
pub const TEMP_PREFIX: &str = ".cloudit-tmp-";

/// The OS path of a manifest path under `root`.
#[must_use]
pub fn local_path(root: &Path, path: &str) -> PathBuf {
    let mut full = root.to_path_buf();
    for part in path.split('/') {
        full.push(part);
    }
    full
}

/// Temp files that are removed on drop unless taken.
#[derive(Debug, Default)]
pub struct TempFiles {
    paths: Vec<PathBuf>,
}

impl TempFiles {
    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    fn forget(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Err(error) = fs::remove_file(path)
                && error.kind() != io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), %error, "could not remove a temp file");
            }
        }
    }
}

fn temp_name() -> Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| Error::Protocol(format!("no randomness: {e}")))?;
    Ok(format!("{TEMP_PREFIX}{}", hex::encode(bytes)))
}

/// Creates a new temp file in `dir`.
fn create_temp(dir: &Path, temps: &mut TempFiles) -> Result<(PathBuf, File)> {
    let path = dir.join(temp_name()?);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::io(format!("creating {}", path.display()), e))?;
    temps.track(path.clone());
    Ok((path, file))
}

/// Where staging finds chunk bytes: the chunk cache, then the local files as scanned.
pub struct ChunkSources {
    root: PathBuf,
    cache_dir: PathBuf,
    /// Chunk → (path, offset) in the scanned local files.
    local: HashMap<ChunkHash, (String, u64)>,
}

impl ChunkSources {
    /// Sources for `root` given its scanned manifest.
    #[must_use]
    pub fn new(root: &Path, cache_dir: &Path, local: &Manifest) -> Self {
        let mut map = HashMap::new();
        for entry in &local.entries {
            let mut offset = 0u64;
            for chunk in &entry.chunks {
                map.entry(chunk.hash)
                    .or_insert_with(|| (entry.path.clone(), offset));
                offset += u64::from(chunk.size);
            }
        }
        Self {
            root: root.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
            local: map,
        }
    }

    /// Whether a local file is expected to hold `hash`.
    #[must_use]
    pub fn has_local(&self, hash: &ChunkHash) -> bool {
        self.local.contains_key(hash)
    }

    /// The cache file of a chunk.
    #[must_use]
    pub fn cache_path(&self, hash: &ChunkHash) -> PathBuf {
        self.cache_dir.join(hash.to_string())
    }

    /// The bytes of `chunk`, verified.
    fn read(&self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        if let Ok(bytes) = fs::read(self.cache_path(&chunk.hash)) {
            if ChunkHash::of(&bytes) == chunk.hash {
                return Ok(bytes);
            }
            tracing::warn!(hash = %chunk.hash, "corrupt chunk in the cache, ignoring it");
        }
        let Some((path, offset)) = self.local.get(&chunk.hash) else {
            return Err(Error::Protocol(format!(
                "chunk {} is neither cached nor local",
                chunk.hash
            )));
        };
        let changed = || Error::LocalChanged { path: path.clone() };
        let mut file = File::open(local_path(&self.root, path)).map_err(|_| changed())?;
        file.seek(SeekFrom::Start(*offset)).map_err(|_| changed())?;
        let mut bytes = vec![0u8; chunk.size as usize];
        file.read_exact(&mut bytes).map_err(|_| changed())?;
        if ChunkHash::of(&bytes) == chunk.hash {
            Ok(bytes)
        } else {
            Err(changed())
        }
    }
}

/// Writes every file of `plan` to a temp file in `root`; the temp paths, by write.
pub fn stage(
    root: &Path,
    plan: &PullPlan,
    sources: &ChunkSources,
    ctx: &OpContext,
    folder: i64,
) -> Result<(TempFiles, Vec<PathBuf>)> {
    let mut temps = TempFiles::default();
    let mut staged = Vec::with_capacity(plan.writes.len());
    let total = plan.writes.len() as u64;
    for (done, write) in plan.writes.iter().enumerate() {
        ctx.check()?;
        ctx.report(
            folder,
            Phase::Staging {
                done: done as u64,
                total,
            },
        );
        let (path, mut file) = create_temp(root, &mut temps)?;
        let context = || format!("writing {}", path.display());
        for chunk in &write.entry.chunks {
            ctx.check()?;
            let bytes = sources.read(chunk)?;
            file.write_all(&bytes)
                .map_err(|e| Error::io(context(), e))?;
        }
        finish_file(&file, &write.entry).map_err(|e| Error::io(context(), e))?;
        staged.push(path);
    }
    ctx.report(folder, Phase::Staging { done: total, total });
    Ok((temps, staged))
}

/// Sets the mtime and executable bit of a fully written file and flushes it to disk.
fn finish_file(file: &File, entry: &ManifestEntry) -> io::Result<()> {
    file.sync_data()?;
    if file.metadata()?.len() != entry.size {
        return Err(io::Error::other("staged file has the wrong size"));
    }
    platform::set_executable(file, entry.executable)?;
    if let Some(time) = platform::system_time(entry.mtime) {
        file.set_modified(time)?;
    }
    file.sync_all()
}

/// Checks that `path` still is what the scan saw (see the module docs).
pub fn recheck(root: &Path, path: &str, expect: &Expect) -> Result<()> {
    let full = local_path(root, path);
    let changed = || Error::LocalChanged {
        path: path.to_owned(),
    };
    let meta = match fs::symlink_metadata(&full) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io(format!("checking {}", full.display()), e)),
    };
    match (expect, meta) {
        (Expect::Absent, None) => Ok(()),
        // An implicit directory (its files were just removed by this pull); removing it
        // fails unless it is empty.
        (Expect::Absent | Expect::EmptyDir, Some(meta)) if meta.is_dir() => Ok(()),
        (Expect::File { state, chunks }, Some(meta)) if meta.is_file() => {
            let mtime = platform::mtime(&meta).map_err(|_| changed())?;
            if meta.len() != state.size || mtime != state.mtime {
                return Err(changed());
            }
            if state.racy && !same_chunks(&full, chunks) {
                return Err(changed());
            }
            Ok(())
        }
        _ => Err(changed()),
    }
}

/// Whether the file's current chunks are `chunks` (false if unreadable).
fn same_chunks(full: &Path, chunks: &[ChunkRef]) -> bool {
    let Ok(file) = File::open(full) else {
        return false;
    };
    let mut expected = chunks.iter();
    for chunk in chunk_reader(file) {
        match (chunk, expected.next()) {
            (Ok(chunk), Some(want)) if chunk.hash == want.hash => {}
            _ => return false,
        }
    }
    expected.next().is_none()
}

/// What an apply changed (also when it stopped early).
#[derive(Debug, Default)]
pub struct Applied {
    /// Index entries of the written files.
    pub written: Vec<(String, IndexEntry)>,
    /// Removed files and directories.
    pub removed: Vec<String>,
    /// Bytes written.
    pub bytes: u64,
}

/// Applies a staged plan (see the module docs). Returns what was done and, separately,
/// how it ended.
pub fn apply(
    root: &Path,
    plan: &PullPlan,
    mut temps: TempFiles,
    staged: &[PathBuf],
    ctx: &OpContext,
    folder: i64,
) -> (Applied, Result<()>) {
    let mut applied = Applied::default();
    let result = apply_inner(root, plan, &mut temps, staged, ctx, folder, &mut applied);
    (applied, result)
}

fn apply_inner(
    root: &Path,
    plan: &PullPlan,
    temps: &mut TempFiles,
    staged: &[PathBuf],
    ctx: &OpContext,
    folder: i64,
    applied: &mut Applied,
) -> Result<()> {
    let total = plan.len() as u64;
    let mut done = 0u64;
    let step = |done: &mut u64| {
        *done += 1;
        ctx.report(folder, Phase::Applying { done: *done, total });
    };
    let mut emptied: BTreeSet<String> = BTreeSet::new();

    // Removals: files first, then empty directories, deepest first.
    let mut removes: Vec<_> = plan.removes.iter().collect();
    removes.sort_by(|a, b| {
        let dir = |e: &Expect| matches!(e, Expect::EmptyDir);
        dir(&a.expect)
            .cmp(&dir(&b.expect))
            .then_with(|| b.path.cmp(&a.path))
    });
    for remove in removes {
        ctx.check()?;
        recheck(root, &remove.path, &remove.expect)?;
        let full = local_path(root, &remove.path);
        let result = if matches!(remove.expect, Expect::EmptyDir) {
            fs::remove_dir(&full)
        } else {
            fs::remove_file(&full)
        };
        match result {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) if matches!(remove.expect, Expect::EmptyDir) => {
                // Not empty any more (or holds ignored files): something new is in it.
                return Err(Error::LocalChanged {
                    path: remove.path.clone(),
                });
            }
            Err(e) => return Err(Error::io(format!("removing {}", full.display()), e)),
        }
        if let Some((parent, _)) = remove.path.rsplit_once('/') {
            emptied.insert(parent.to_owned());
        }
        applied.removed.push(remove.path.clone());
        step(&mut done);
    }

    // Directories, then files, in path order.
    for dir in &plan.mkdirs {
        ctx.check()?;
        let full = local_path(root, dir);
        fs::create_dir_all(&full)
            .map_err(|e| Error::io(format!("creating {}", full.display()), e))?;
        step(&mut done);
    }
    for (write, temp) in plan.writes.iter().zip(staged) {
        ctx.check()?;
        recheck(root, &write.entry.path, &write.expect)?;
        let target = local_path(root, &write.entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("creating {}", parent.display()), e))?;
        }
        if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
            fs::remove_dir(&target).map_err(|_| Error::LocalChanged {
                path: write.entry.path.clone(),
            })?;
        }
        replace(temp, &target, temps)?;
        temps.forget(temp);
        let meta = fs::metadata(&target)
            .map_err(|e| Error::io(format!("reading {}", target.display()), e))?;
        applied.written.push((
            write.entry.path.clone(),
            IndexEntry {
                size: meta.len(),
                mtime: platform::mtime(&meta)
                    .map_err(|e| Error::io(format!("reading {}", target.display()), e))?,
                executable: write.entry.executable,
                chunks: write.entry.chunks.clone(),
            },
        ));
        applied.bytes += write.entry.size;
        step(&mut done);
    }

    // Directories left empty by removals that the head doesn't have any more.
    let keep: BTreeSet<&str> = plan
        .writes
        .iter()
        .map(|w| w.entry.path.as_str())
        .chain(plan.mkdirs.iter().map(String::as_str))
        .collect();
    for dir in emptied.iter().rev() {
        let mut current = dir.as_str();
        loop {
            let needed = keep
                .iter()
                .any(|p| *p == current || p.starts_with(&format!("{current}/")));
            if needed || fs::remove_dir(local_path(root, current)).is_err() {
                break;
            }
            match current.rsplit_once('/') {
                Some((parent, _)) => current = parent,
                None => break,
            }
        }
    }
    Ok(())
}

/// Renames `temp` over `target`; across volumes, copies next to the target first.
fn replace(temp: &Path, target: &Path, temps: &mut TempFiles) -> Result<()> {
    match fs::rename(temp, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            let dir = target
                .parent()
                .ok_or_else(|| Error::io("no parent directory", io::Error::other("root")))?;
            let (sibling, mut file) = create_temp(dir, temps)?;
            let mut source = File::open(temp)
                .map_err(|e| Error::io(format!("reading {}", temp.display()), e))?;
            io::copy(&mut source, &mut file)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io(format!("copying to {}", sibling.display()), e))?;
            let modified = source.metadata().and_then(|m| m.modified());
            if let Ok(time) = modified {
                file.set_modified(time)
                    .map_err(|e| Error::io(format!("writing {}", sibling.display()), e))?;
            }
            fs::rename(&sibling, target)
                .map_err(|e| Error::io(format!("replacing {}", target.display()), e))?;
            temps.forget(&sibling);
            Ok(())
        }
        Err(e) => Err(Error::io(format!("replacing {}", target.display()), e)),
    }
}
