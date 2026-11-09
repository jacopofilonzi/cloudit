//! One recursive file watcher (`notify`) per automatic folder. Its callback runs on
//! notify's thread and only filters: an event that may have changed synced content
//! sends the folder's id to the scheduler, which debounces. Reads (`Access`) and the
//! engine's temp files (`.cloudit-tmp-*`) are dropped; so are events on ignored paths.
//! An error or a "rescan" event (the OS dropped events) counts as a change. Events
//! caused by the engine's own pulls are dropped by the scheduler (it knows when an
//! operation runs on the folder).

use std::path::{Path, PathBuf};

use cloudit_client::scan::IgnoreRules;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc::UnboundedSender;

use super::Input;

/// A running watcher; dropping it stops watching.
pub struct FolderWatcher {
    /// What it watches, to notice a relocation or new ignore patterns.
    pub path: PathBuf,
    pub ignore: Vec<String>,
    _watcher: RecommendedWatcher,
}

impl FolderWatcher {
    /// Watches `path` recursively for `folder`. Blocking (the OS may walk the tree).
    pub fn start(
        folder: i64,
        path: &Path,
        ignore: &[String],
        tx: UnboundedSender<Input>,
    ) -> Result<Self, String> {
        let rules = IgnoreRules::new(ignore).map_err(|e| e.to_string())?;
        let root = path.to_path_buf();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let changed = match &event {
                Ok(event) => relevant(event, &root, &rules),
                Err(error) => {
                    tracing::debug!(folder, %error, "file watcher error: treated as a change");
                    true
                }
            };
            if changed {
                let _closed = tx.send(Input::Files(folder));
            }
        })
        .map_err(|e| format!("creating a watcher: {e}"))?;
        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| format!("watching {}: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            ignore: ignore.to_vec(),
            _watcher: watcher,
        })
    }
}

/// Whether `event` may have changed content of the folder at `root`.
fn relevant(event: &Event, root: &Path, rules: &IgnoreRules) -> bool {
    if event.need_rescan() {
        return true;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    // No paths: the OS couldn't say what changed.
    event.paths.is_empty()
        || event.paths.iter().any(|path| match relative(root, path) {
            Some(rel) if rel.is_empty() => true,
            // Directories are not known here: check both ways, so an ignored `logs/`
            // drops `logs/x` but a file named like a directory pattern still counts.
            Some(rel) => !rules.is_ignored(&rel, false) || !rules.is_ignored(&rel, true),
            None => false,
        })
}

/// `path` relative to `root` with `/` separators (`None` when outside).
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, Flag, ModifyKind};

    fn event(kind: EventKind, paths: &[&Path]) -> Event {
        let mut event = Event::new(kind);
        for path in paths {
            event = event.add_path(path.to_path_buf());
        }
        event
    }

    #[test]
    fn only_content_changes_count() {
        let root = std::env::temp_dir().join("cloudit-watch");
        let rules = IgnoreRules::new(&["*.log".to_owned(), "cache/".to_owned()]).unwrap();
        let file = root.join("saves").join("slot1.sav");
        let modify = EventKind::Modify(ModifyKind::Any);
        assert!(relevant(&event(modify, &[&file]), &root, &rules));
        assert!(relevant(
            &event(EventKind::Create(CreateKind::File), &[&file]),
            &root,
            &rules
        ));
        assert!(!relevant(
            &event(EventKind::Access(AccessKind::Any), &[&file]),
            &root,
            &rules
        ));
        let temp = root.join(".cloudit-tmp-0123456789abcdef");
        assert!(!relevant(&event(modify, &[&temp]), &root, &rules));
        assert!(!relevant(
            &event(modify, &[&root.join("game.log")]),
            &root,
            &rules
        ));
        assert!(!relevant(
            &event(modify, &[&root.join("cache").join("x")]),
            &root,
            &rules
        ));
        // One relevant path among ignored ones is enough.
        assert!(relevant(&event(modify, &[&temp, &file]), &root, &rules));
        // Outside the folder; no paths; the OS lost events.
        assert!(!relevant(
            &event(modify, &[&std::env::temp_dir().join("other")]),
            &root,
            &rules
        ));
        assert!(relevant(&event(modify, &[]), &root, &rules));
        assert!(relevant(
            &event(EventKind::Access(AccessKind::Any), &[&file]).set_flag(Flag::Rescan),
            &root,
            &rules
        ));
        assert!(relevant(&event(modify, &[&root]), &root, &rules));
    }
}
