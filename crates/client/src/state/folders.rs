//! The `folders` table: linked folders and their sync base.

use std::path::PathBuf;

use rusqlite::{OptionalExtension as _, Row, params};
use serde::Serialize;

use super::{Db, now_rfc3339};
use crate::error::Result;
use crate::sync::status::SyncState;

/// A local folder linked to a server folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LinkedFolder {
    /// Local id (stable while linked).
    pub id: i64,
    pub local_path: PathBuf,
    pub scope_id: String,
    /// The server folder id.
    pub folder_id: String,
    /// Cached names (refreshed by [`crate::Engine::refresh_names`]).
    pub scope_name: String,
    pub folder_name: String,
    pub emoji: Option<String>,
    /// Ignore patterns (see [`crate::scan::IgnoreRules`]).
    pub ignore: Vec<String>,
    /// No automatic sync (e.g. a conflict decision was postponed).
    pub paused: bool,
    /// The last snapshot this device and the server agreed on.
    pub base_snapshot_id: Option<String>,
    /// A conflict snapshot uploaded by this device, waiting for a decision.
    pub pending_conflict_id: Option<String>,
    pub last_state: Option<SyncState>,
    pub last_error: Option<String>,
    pub last_sync_at: Option<String>,
    pub created_at: String,
    /// Synced automatically by the desktop app (see [`crate::AutoSyncSettings`]).
    pub auto_sync: bool,
    /// Seconds without file changes before an automatic sync starts.
    pub quiet_secs: u32,
    /// The program whose files these are (an executable name): while it runs nothing is
    /// pulled into the folder, and the folder is synced when it exits.
    pub process_name: Option<String>,
}

/// What [`Db::insert_folder`] needs.
#[derive(Debug, Clone)]
pub struct NewFolder<'a> {
    pub local_path: &'a str,
    pub scope_id: &'a str,
    pub folder_id: &'a str,
    pub scope_name: &'a str,
    pub folder_name: &'a str,
    pub emoji: Option<&'a str>,
    pub ignore: &'a [String],
}

const COLUMNS: &str = "id, local_path, scope_id, folder_id, scope_name, folder_name, emoji,
    ignore_patterns, paused, base_snapshot_id, pending_conflict_id, last_state, last_error,
    last_sync_at, created_at, auto_sync, quiet_secs, process_name";

fn from_row(r: &Row<'_>) -> rusqlite::Result<LinkedFolder> {
    let path: String = r.get(1)?;
    let ignore: String = r.get(7)?;
    let state: Option<String> = r.get(11)?;
    Ok(LinkedFolder {
        id: r.get(0)?,
        local_path: PathBuf::from(path),
        scope_id: r.get(2)?,
        folder_id: r.get(3)?,
        scope_name: r.get(4)?,
        folder_name: r.get(5)?,
        emoji: r.get(6)?,
        ignore: split_patterns(&ignore),
        paused: r.get(8)?,
        base_snapshot_id: r.get(9)?,
        pending_conflict_id: r.get(10)?,
        last_state: state.as_deref().and_then(SyncState::parse),
        last_error: r.get(12)?,
        last_sync_at: r.get(13)?,
        created_at: r.get(14)?,
        auto_sync: r.get(15)?,
        quiet_secs: r.get(16)?,
        process_name: r.get(17)?,
    })
}

fn split_patterns(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn join_patterns(patterns: &[String]) -> String {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A snapshot id with its stored (zstd) manifest bytes.
pub type StoredSnapshot = (String, Vec<u8>);

impl Db {
    /// Every linked folder, by local id.
    pub fn folders(&self) -> Result<Vec<LinkedFolder>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLUMNS} FROM folders ORDER BY id"))?;
            stmt.query_map([], from_row)?.collect()
        })
    }

    /// One linked folder.
    pub fn folder(&self, id: i64) -> Result<Option<LinkedFolder>> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM folders WHERE id = ?1"),
                [id],
                from_row,
            )
            .optional()
        })
    }

    /// Links a folder; returns its local id.
    pub fn insert_folder(&self, new: &NewFolder<'_>) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO folders (local_path, scope_id, folder_id, scope_name, folder_name,
                                      emoji, ignore_patterns, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    new.local_path,
                    new.scope_id,
                    new.folder_id,
                    new.scope_name,
                    new.folder_name,
                    new.emoji,
                    join_patterns(new.ignore),
                    now_rfc3339(),
                ],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    /// Unlinks a folder (its index goes with it).
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM folders WHERE id = ?1", [id])
                .map(drop)
        })
    }

    /// Unlinks every folder (another account signed in).
    pub fn delete_all_folders(&self) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM folders", []).map(drop))
    }

    pub fn set_local_path(&self, id: i64, path: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET local_path = ?2 WHERE id = ?1",
                params![id, path],
            )
            .map(drop)
        })
    }

    pub fn set_ignore(&self, id: i64, patterns: &[String]) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET ignore_patterns = ?2 WHERE id = ?1",
                params![id, join_patterns(patterns)],
            )
            .map(drop)
        })
    }

    pub fn set_paused(&self, id: i64, paused: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET paused = ?2 WHERE id = ?1",
                params![id, paused],
            )
            .map(drop)
        })
    }

    /// Stores the automatic sync settings (validated by the engine).
    pub fn set_auto_sync(
        &self,
        id: i64,
        enabled: bool,
        quiet_secs: u32,
        process_name: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET auto_sync = ?2, quiet_secs = ?3, process_name = ?4
                 WHERE id = ?1",
                params![id, enabled, quiet_secs, process_name],
            )
            .map(drop)
        })
    }

    pub fn set_names(
        &self,
        id: i64,
        scope_name: &str,
        folder_name: &str,
        emoji: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET scope_name = ?2, folder_name = ?3, emoji = ?4 WHERE id = ?1",
                params![id, scope_name, folder_name, emoji],
            )
            .map(drop)
        })
    }

    /// The base: last synced snapshot and its stored manifest.
    pub fn base(&self, id: i64) -> Result<Option<StoredSnapshot>> {
        self.snapshot_columns(id, "base_snapshot_id", "base_manifest")
    }

    /// Moves the base (after a whole push or pull succeeded).
    pub fn set_base(&self, id: i64, snapshot_id: &str, stored_manifest: &[u8]) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET base_snapshot_id = ?2, base_manifest = ?3,
                                    last_sync_at = ?4
                 WHERE id = ?1",
                params![id, snapshot_id, stored_manifest, now_rfc3339()],
            )
            .map(drop)
        })
    }

    /// The conflict snapshot this device uploaded and its manifest.
    pub fn pending_conflict(&self, id: i64) -> Result<Option<StoredSnapshot>> {
        self.snapshot_columns(id, "pending_conflict_id", "pending_conflict_manifest")
    }

    pub fn set_pending_conflict(&self, id: i64, conflict: Option<(&str, &[u8])>) -> Result<()> {
        let (snapshot, manifest) = conflict.unzip();
        self.with(|c| {
            c.execute(
                "UPDATE folders SET pending_conflict_id = ?2, pending_conflict_manifest = ?3
                 WHERE id = ?1",
                params![id, snapshot, manifest],
            )
            .map(drop)
        })
    }

    /// The last downloaded head manifest.
    pub fn head_cache(&self, id: i64) -> Result<Option<StoredSnapshot>> {
        self.snapshot_columns(id, "head_cache_id", "head_cache_manifest")
    }

    pub fn set_head_cache(&self, id: i64, snapshot_id: &str, stored_manifest: &[u8]) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET head_cache_id = ?2, head_cache_manifest = ?3 WHERE id = ?1",
                params![id, snapshot_id, stored_manifest],
            )
            .map(drop)
        })
    }

    /// Records the last computed state and error.
    pub fn set_last_state(&self, id: i64, state: SyncState, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE folders SET last_state = ?2, last_error = ?3 WHERE id = ?1",
                params![id, state.as_str(), error],
            )
            .map(drop)
        })
    }

    fn snapshot_columns(
        &self,
        id: i64,
        id_column: &str,
        manifest_column: &str,
    ) -> Result<Option<StoredSnapshot>> {
        let found: Option<(Option<String>, Option<Vec<u8>>)> = self.with(|c| {
            c.query_row(
                &format!("SELECT {id_column}, {manifest_column} FROM folders WHERE id = ?1"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })?;
        Ok(match found {
            Some((Some(snapshot), Some(manifest))) => Some((snapshot, manifest)),
            _ => None,
        })
    }
}
