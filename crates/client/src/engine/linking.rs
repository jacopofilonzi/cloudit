//! Linking local folders to server folders.
//!
//! A link is local state only: unlinking or relinking never touches files. The local
//! path is created if missing, stored canonical, and refused when it is (inside) the
//! engine's data directory, already linked, or nested in / around another linked
//! folder; a server folder is linked at most once per device. What the first sync does
//! follows from the status: an empty local folder pulls the server's content, a new
//! server folder receives the local content, identical content is simply in sync, and
//! different content on both sides is a conflict (the local state is uploaded as a
//! conflict snapshot and the user decides).
//!
//! Also the link's settings: ignore patterns, pause, and the automatic sync settings
//! ([`AutoSyncSettings`], plus the device-wide switch in `meta`), which the engine only
//! stores and validates.

use std::ops::RangeInclusive;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Engine;
use crate::error::{Error, LinkRefusal, Result};
use crate::platform;
use crate::progress::OpContext;
use crate::scan::IgnoreRules;
use crate::state::activity::NewActivity;
use crate::state::folders::{LinkedFolder, NewFolder};

/// `meta` key of the global automatic sync switch (`on` / `off`; missing = on).
const AUTO_SYNC_KEY: &str = "auto_sync";
/// Allowed quiet periods, in seconds.
pub const QUIET_SECS: RangeInclusive<u32> = 5..=3600;
/// Longest associated process name, in characters.
pub const MAX_PROCESS_NAME: usize = 255;

/// A folder's automatic sync settings. The engine only stores them: the desktop app
/// schedules the runs (always through [`Engine::sync_with`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AutoSyncSettings {
    pub enabled: bool,
    /// Seconds without file changes before a run ([`QUIET_SECS`]).
    pub quiet_secs: u32,
    /// The executable name of the program that uses the folder (e.g. `game.exe`):
    /// trimmed, 1..=255 characters, no path separators or control characters. Empty or
    /// `None` clears it.
    pub process_name: Option<String>,
}

impl AutoSyncSettings {
    /// The process name to store (`None` when cleared).
    fn validated_process(&self) -> Result<Option<String>> {
        let Some(name) = self.process_name.as_deref().map(str::trim) else {
            return Ok(None);
        };
        if name.is_empty() {
            return Ok(None);
        }
        if name.chars().count() > MAX_PROCESS_NAME
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput(format!(
                "process name {name:?}: an executable name, at most {MAX_PROCESS_NAME} \
                 characters, without path separators"
            )));
        }
        Ok(Some(name.to_owned()))
    }
}

impl Engine {
    /// Every linked folder.
    pub fn folders(&self) -> Result<Vec<LinkedFolder>> {
        self.db().folders()
    }

    /// One linked folder.
    pub fn folder(&self, id: i64) -> Result<LinkedFolder> {
        self.db().folder(id)?.ok_or(Error::NotLinked(id))
    }

    /// Links an existing server folder to `local_path` (then call [`Engine::sync`]).
    pub async fn link_folder(
        &self,
        folder_id: &str,
        local_path: &Path,
        ignore: &[String],
        ctx: &OpContext,
    ) -> Result<LinkedFolder> {
        IgnoreRules::new(ignore)?;
        let path = self.prepare_path(local_path, None)?;
        if self
            .db()
            .folders()?
            .iter()
            .any(|f| f.folder_id == folder_id)
        {
            return Err(Error::LinkRefused {
                path,
                reason: LinkRefusal::FolderAlreadyLinked,
            });
        }
        let api = self.api()?;
        let info = api.folder(folder_id, ctx).await?;
        let scope = api.scope(&info.scope_id, ctx).await?;
        let id = self.db().insert_folder(&NewFolder {
            local_path: &path,
            scope_id: &info.scope_id,
            folder_id: &info.id,
            scope_name: &scope.name,
            folder_name: &info.name,
            emoji: info.emoji.as_deref(),
            ignore,
        })?;
        self.log(&NewActivity {
            folder_id: Some(id),
            folder_name: Some(&info.name),
            action: "link",
            outcome: "ok",
            ..NewActivity::default()
        });
        self.folder(id)
    }

    /// Creates a server folder in `scope_id` and links it to `local_path` (the first
    /// sync uploads the local content).
    pub async fn create_and_link(
        &self,
        scope_id: &str,
        name: &str,
        emoji: Option<&str>,
        local_path: &Path,
        ignore: &[String],
        ctx: &OpContext,
    ) -> Result<LinkedFolder> {
        IgnoreRules::new(ignore)?;
        // Refuse a bad path before creating anything on the server.
        self.prepare_path(local_path, None)?;
        let created = self.create_folder(scope_id, name, emoji, ctx).await?;
        self.link_folder(&created.id, local_path, ignore, ctx).await
    }

    /// Unlinks a folder: forgets its state; the local files stay as they are.
    pub async fn unlink(&self, id: i64) -> Result<()> {
        let _guard = self.lock(id).await?;
        let link = self.folder(id)?;
        self.db().delete_folder(id)?;
        self.log(&NewActivity {
            folder_id: Some(id),
            folder_name: Some(&link.folder_name),
            action: "unlink",
            outcome: "ok",
            ..NewActivity::default()
        });
        Ok(())
    }

    /// Points a link at another local path (e.g. the folder was moved). The base is
    /// kept: if the new path holds other content, the next status shows it.
    pub async fn relocate(&self, id: i64, local_path: &Path) -> Result<LinkedFolder> {
        let _guard = self.lock(id).await?;
        self.folder(id)?;
        let path = self.prepare_path(local_path, Some(id))?;
        self.db().set_local_path(id, &path)?;
        self.folder(id)
    }

    /// Replaces the ignore patterns (validated first).
    pub async fn set_ignore(&self, id: i64, patterns: &[String]) -> Result<LinkedFolder> {
        IgnoreRules::new(patterns)?;
        let _guard = self.lock(id).await?;
        self.folder(id)?;
        self.db().set_ignore(id, patterns)?;
        self.folder(id)
    }

    /// Pauses or resumes automatic sync of a folder.
    pub fn set_paused(&self, id: i64, paused: bool) -> Result<LinkedFolder> {
        self.folder(id)?;
        self.db().set_paused(id, paused)?;
        self.folder(id)
    }

    /// Changes a folder's automatic sync settings (validated: see [`AutoSyncSettings`]).
    pub fn set_auto_sync(&self, id: i64, settings: &AutoSyncSettings) -> Result<LinkedFolder> {
        let process = settings.validated_process()?;
        if !QUIET_SECS.contains(&settings.quiet_secs) {
            return Err(Error::InvalidInput(format!(
                "quiet period {} s: allowed {}..={} s",
                settings.quiet_secs,
                QUIET_SECS.start(),
                QUIET_SECS.end()
            )));
        }
        self.folder(id)?;
        self.db().set_auto_sync(
            id,
            settings.enabled,
            settings.quiet_secs,
            process.as_deref(),
        )?;
        self.folder(id)
    }

    /// Whether automatic sync is on for this device (the global switch; default on).
    pub fn auto_sync_enabled(&self) -> Result<bool> {
        Ok(self.db().meta(AUTO_SYNC_KEY)?.as_deref() != Some("off"))
    }

    /// Turns automatic sync on or off for every folder of this device.
    pub fn set_auto_sync_enabled(&self, enabled: bool) -> Result<()> {
        self.db()
            .set_meta(AUTO_SYNC_KEY, if enabled { "on" } else { "off" })
    }

    /// Refreshes the cached scope and folder names (and emoji) from the server.
    pub async fn refresh_names(&self, ctx: &OpContext) -> Result<Vec<LinkedFolder>> {
        let api = self.api()?;
        for link in self.db().folders()? {
            match api.folder(&link.folder_id, ctx).await {
                Ok(info) => {
                    let scope = api.scope(&info.scope_id, ctx).await?;
                    self.db()
                        .set_names(link.id, &scope.name, &info.name, info.emoji.as_deref())?;
                }
                Err(e) if e.kind() == crate::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        self.db().folders()
    }

    /// Validates a local path for linking (creating it if missing); its stored form.
    fn prepare_path(&self, path: &Path, except: Option<i64>) -> Result<String> {
        let shown = path.display().to_string();
        let refuse = |reason| Error::LinkRefused {
            path: shown.clone(),
            reason,
        };
        match std::fs::metadata(path) {
            Ok(meta) if !meta.is_dir() => return Err(refuse(LinkRefusal::NotADirectory)),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(path)
                    .map_err(|e| Error::io(format!("creating {shown}"), e))?;
            }
            Err(e) => return Err(Error::io(format!("reading {shown}"), e)),
        }
        let canonical =
            platform::canonical(path).map_err(|e| Error::io(format!("reading {shown}"), e))?;
        let text = canonical
            .to_str()
            .ok_or_else(|| refuse(LinkRefusal::NotUtf8))?
            .to_owned();
        let key = platform::path_key(&canonical);
        let data = platform::path_key(self.data_dir());
        if key.starts_with(&data) || data.starts_with(&key) {
            return Err(refuse(LinkRefusal::DataDir));
        }
        for other in self.db().folders()? {
            if Some(other.id) == except {
                continue;
            }
            let other_key = platform::path_key(&other.local_path);
            if other_key == key {
                return Err(refuse(LinkRefusal::AlreadyLinked));
            }
            if other_key.starts_with(&key) || key.starts_with(&other_key) {
                return Err(refuse(LinkRefusal::Nested));
            }
        }
        Ok(text)
    }
}
