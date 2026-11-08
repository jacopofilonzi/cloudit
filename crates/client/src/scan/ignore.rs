//! Ignore patterns of a linked folder (a small, documented subset of `.gitignore`).
//!
//! - One pattern per entry; blank entries and entries starting with `#` are skipped.
//! - Patterns match relative paths with `/` separators. `*` and `?` never cross a `/`;
//!   `**` matches any number of directories; `[abc]` / `{a,b}` work as in `globset`.
//! - A pattern without `/` matches a name at any depth: `*.tmp`, `session.lock`.
//! - A pattern with a `/` in the middle (or a leading `/`, which is dropped) is anchored
//!   at the folder root: `saves/backup*`, `/config.ini`.
//! - A trailing `/` matches directories only: `logs/`. An ignored directory ignores
//!   everything inside it.
//! - Negation (`!pattern`) is not supported (refused).
//! - Matching ignores letter case where the file system does (Windows, macOS).
//!
//! The engine's own temp files (`.cloudit-tmp-*`, see `sync::stage`) are always ignored.
//!
//! Ignored paths are never uploaded, and a pull never writes, overwrites or deletes
//! them; entries the server has under ignored paths are carried over unchanged into
//! this device's uploads (ignoring is local: it never deletes files for other devices).

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::{Error, Result};
use crate::sync::stage::TEMP_PREFIX;

/// Compiled ignore patterns.
#[derive(Debug, Clone)]
pub struct IgnoreRules {
    any: GlobSet,
    dirs: GlobSet,
}

impl Default for IgnoreRules {
    fn default() -> Self {
        Self {
            any: GlobSet::empty(),
            dirs: GlobSet::empty(),
        }
    }
}

impl IgnoreRules {
    /// Compiles `patterns` (see the module docs).
    pub fn new(patterns: &[String]) -> Result<Self> {
        let mut any = GlobSetBuilder::new();
        let mut dirs = GlobSetBuilder::new();
        for raw in patterns {
            let pattern = raw.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                continue;
            }
            if pattern.starts_with('!') {
                return Err(Error::InvalidInput(format!(
                    "ignore pattern {pattern:?}: negation is not supported"
                )));
            }
            let (pattern, dir_only) = match pattern.strip_suffix('/') {
                Some(rest) => (rest, true),
                None => (pattern, false),
            };
            let anchored = pattern.starts_with('/') || pattern.contains('/');
            let pattern = pattern.trim_start_matches('/');
            if pattern.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "ignore pattern {raw:?} matches nothing"
                )));
            }
            let full = if anchored {
                pattern.to_owned()
            } else {
                format!("**/{pattern}")
            };
            let glob = GlobBuilder::new(&full)
                .literal_separator(true)
                .case_insensitive(crate::platform::case_insensitive_names())
                .build()
                .map_err(|e| Error::InvalidInput(format!("ignore pattern {raw:?}: {e}")))?;
            if dir_only {
                dirs.add(glob);
            } else {
                any.add(glob);
            }
        }
        Ok(Self {
            any: any
                .build()
                .map_err(|e| Error::InvalidInput(format!("ignore patterns: {e}")))?,
            dirs: dirs
                .build()
                .map_err(|e| Error::InvalidInput(format!("ignore patterns: {e}")))?,
        })
    }

    /// Whether the entry at `path` itself is ignored (its parents are not checked: the
    /// scanner never enters an ignored directory).
    #[must_use]
    pub fn matches(&self, path: &str, is_dir: bool) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.starts_with(TEMP_PREFIX) {
            return true;
        }
        self.any.is_match(path) || (is_dir && self.dirs.is_match(path))
    }

    /// Whether `path` or any of its parent directories is ignored (for paths from a
    /// manifest).
    #[must_use]
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let mut end = 0;
        while let Some(pos) = path.get(end..).and_then(|rest| rest.find('/')) {
            let parent = path.get(..end + pos).unwrap_or(path);
            if self.matches(parent, true) {
                return true;
            }
            end += pos + 1;
        }
        self.matches(path, is_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(patterns: &[&str]) -> IgnoreRules {
        IgnoreRules::new(&patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn name_patterns_match_at_any_depth() {
        let r = rules(&["*.tmp", "session.lock", "# comment", ""]);
        assert!(r.matches("a.tmp", false));
        assert!(r.matches("x/y/a.tmp", false));
        assert!(r.matches("deep/session.lock", false));
        assert!(!r.matches("a.tmpx", false));
        assert!(!r.matches("session.lock.bak", false));
    }

    #[test]
    fn dir_patterns_match_dirs_only() {
        let r = rules(&["logs/"]);
        assert!(r.matches("logs", true));
        assert!(r.matches("a/logs", true));
        assert!(!r.matches("logs", false));
        assert!(r.is_ignored("logs/today.txt", false));
        assert!(r.is_ignored("a/logs/x/y", false));
        assert!(!r.is_ignored("logsx/y", false));
    }

    #[test]
    fn slashes_anchor_patterns() {
        let r = rules(&["saves/backup*", "/config.ini"]);
        assert!(r.matches("saves/backup1", false));
        assert!(!r.matches("x/saves/backup1", false));
        assert!(r.matches("config.ini", false));
        assert!(!r.matches("x/config.ini", false));
        // `*` doesn't cross directories; `**` does.
        assert!(!rules(&["a/*"]).matches("a/b/c", false));
        assert!(rules(&["a/**"]).matches("a/b/c", false));
    }

    #[test]
    fn temp_files_are_always_ignored() {
        let r = IgnoreRules::default();
        assert!(r.matches(".cloudit-tmp-0123", false));
        assert!(r.is_ignored("x/.cloudit-tmp-0123", false));
        assert!(!r.matches("cloudit-tmp", false));
    }

    #[test]
    fn negation_and_bad_globs_are_refused() {
        assert!(IgnoreRules::new(&["!keep.txt".to_owned()]).is_err());
        assert!(IgnoreRules::new(&["a[".to_owned()]).is_err());
        assert!(IgnoreRules::new(&["/".to_owned()]).is_err());
    }
}
