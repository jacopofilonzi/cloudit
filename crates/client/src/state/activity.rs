//! The `activity` table: what the engine did, for the UI's activity list. Bounded: only
//! the newest [`MAX_ROWS`] entries are kept.

use rusqlite::params;
use serde::Serialize;

use super::{Db, now_rfc3339};
use crate::error::Result;

/// Entries kept.
pub const MAX_ROWS: i64 = 1000;

/// One logged operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ActivityEntry {
    pub id: i64,
    /// RFC 3339 UTC.
    pub at: String,
    /// The linked folder (may be unlinked since).
    pub folder_id: Option<i64>,
    pub folder_name: Option<String>,
    /// `push`, `pull`, `conflict`, `keep_local`, `keep_server`, `link`, `unlink`, …
    pub action: String,
    /// `ok` or `error` (or `skipped`).
    pub outcome: String,
    /// Files written, deleted or uploaded.
    pub files: u64,
    /// Bytes transferred.
    pub bytes: u64,
    /// The snapshot created or applied.
    pub snapshot_id: Option<String>,
    /// [`crate::ErrorKind`] name when `outcome` is `error`.
    pub error_kind: Option<String>,
    /// English error message (logs).
    pub error: Option<String>,
}

/// What to log.
#[derive(Debug, Clone, Default)]
pub struct NewActivity<'a> {
    pub folder_id: Option<i64>,
    pub folder_name: Option<&'a str>,
    pub action: &'a str,
    pub outcome: &'a str,
    pub files: u64,
    pub bytes: u64,
    pub snapshot_id: Option<&'a str>,
    pub error_kind: Option<&'a str>,
    pub error: Option<&'a str>,
}

impl Db {
    /// Appends an entry and drops the oldest beyond [`MAX_ROWS`].
    pub fn log_activity(&self, entry: &NewActivity<'_>) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO activity (at, folder_id, folder_name, action, outcome, files, bytes,
                                       snapshot_id, error_kind, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    now_rfc3339(),
                    entry.folder_id,
                    entry.folder_name,
                    entry.action,
                    entry.outcome,
                    i64::try_from(entry.files).unwrap_or(i64::MAX),
                    i64::try_from(entry.bytes).unwrap_or(i64::MAX),
                    entry.snapshot_id,
                    entry.error_kind,
                    entry.error,
                ],
            )?;
            let last = tx.last_insert_rowid();
            tx.execute("DELETE FROM activity WHERE id <= ?1", [last - MAX_ROWS])?;
            tx.commit()
        })
    }

    /// The newest `limit` entries, newest first (optionally of one folder).
    pub fn activity(&self, folder: Option<i64>, limit: u32) -> Result<Vec<ActivityEntry>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, at, folder_id, folder_name, action, outcome, files, bytes,
                        snapshot_id, error_kind, error
                 FROM activity WHERE ?1 IS NULL OR folder_id = ?1
                 ORDER BY id DESC LIMIT ?2",
            )?;
            stmt.query_map(params![folder, limit], |r| {
                let files: i64 = r.get(6)?;
                let bytes: i64 = r.get(7)?;
                Ok(ActivityEntry {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    folder_id: r.get(2)?,
                    folder_name: r.get(3)?,
                    action: r.get(4)?,
                    outcome: r.get(5)?,
                    files: u64::try_from(files).unwrap_or(0),
                    bytes: u64::try_from(bytes).unwrap_or(0),
                    snapshot_id: r.get(8)?,
                    error_kind: r.get(9)?,
                    error: r.get(10)?,
                })
            })?
            .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("s.db")).unwrap();
        for _ in 0..(MAX_ROWS + 5) {
            db.log_activity(&NewActivity {
                action: "push",
                outcome: "ok",
                ..NewActivity::default()
            })
            .unwrap();
        }
        let all = db.activity(None, 5000).unwrap();
        assert_eq!(i64::try_from(all.len()).unwrap(), MAX_ROWS);
        assert!(all[0].id > all[1].id);
        assert!(db.activity(Some(1), 10).unwrap().is_empty());
    }
}
