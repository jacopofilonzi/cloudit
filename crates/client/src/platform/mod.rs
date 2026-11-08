//! Everything that depends on the operating system lives here (and in `unix.rs` /
//! `windows.rs`): default directories, the executable bit, private files, path
//! comparison, opening the browser. No other module branches on the OS.

use std::fs::{self, File, Metadata};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cloudit_core::manifest::Timestamp;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

/// Directory name of the engine's data under the OS data directory.
const APP_DIR: &str = "cloudit";

/// The default data directory of the engine (callers like the Tauri app may pass their
/// own): `%LOCALAPPDATA%\cloudit` on Windows, `$XDG_DATA_HOME/cloudit` (default
/// `~/.local/share/cloudit`) on Linux, `~/Library/Application Support/cloudit` on macOS.
/// `None` when the environment gives no home directory.
#[must_use]
pub fn default_data_dir() -> Option<PathBuf> {
    os::data_root().map(|root| root.join(APP_DIR))
}

/// The OS name sent at device sign-in (`windows`, `linux`, `macos`).
#[must_use]
pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// A default device name: the computer's host name, or the OS name.
#[must_use]
pub fn device_name() -> String {
    os::host_name()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| os_name().to_owned())
}

/// Whether files carry an executable bit here. Where they don't (Windows), the scanner
/// keeps the flag of the last synced version, so pulling an executable file doesn't look
/// like a local change forever.
#[must_use]
pub fn supports_exec_bit() -> bool {
    cfg!(unix)
}

/// Whether file names differ only by letter case count as the same file here (Windows
/// and macOS by default): ignore patterns and path comparisons follow it.
#[must_use]
pub fn case_insensitive_names() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// The executable bit of a file (always `false` without [`supports_exec_bit`]).
#[must_use]
pub fn is_executable(meta: &Metadata) -> bool {
    os::is_executable(meta)
}

/// Sets or clears the executable bit (a no-op without [`supports_exec_bit`]).
pub fn set_executable(file: &File, executable: bool) -> io::Result<()> {
    os::set_executable(file, executable)
}

/// The modification time of `meta` as a manifest [`Timestamp`].
pub fn mtime(meta: &Metadata) -> io::Result<Timestamp> {
    Ok(timestamp(meta.modified()?))
}

/// A [`SystemTime`] as a manifest [`Timestamp`] (times before 1970 included).
#[must_use]
pub fn timestamp(time: SystemTime) -> Timestamp {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => Timestamp {
            secs: i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
            nanos: after.subsec_nanos(),
        },
        Err(before) => {
            let before = before.duration();
            let secs = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
            if before.subsec_nanos() == 0 {
                Timestamp {
                    secs: -secs,
                    nanos: 0,
                }
            } else {
                Timestamp {
                    secs: -secs - 1,
                    nanos: 1_000_000_000 - before.subsec_nanos(),
                }
            }
        }
    }
}

/// A manifest [`Timestamp`] as a [`SystemTime`] (`None` if not representable).
#[must_use]
pub fn system_time(ts: Timestamp) -> Option<SystemTime> {
    if ts.secs >= 0 {
        let secs = u64::try_from(ts.secs).ok()?;
        UNIX_EPOCH.checked_add(Duration::new(secs, ts.nanos))
    } else {
        let secs = u64::try_from(ts.secs.checked_neg()?).ok()?;
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(secs))?
            .checked_add(Duration::from_nanos(u64::from(ts.nanos)))
    }
}

/// Writes `bytes` to `path` readable only by the current user (0600 on Unix; on Windows
/// the per-user data directory's ACL already restricts it), through a temp file and a
/// rename so a crash never leaves half a file.
pub fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the path has no parent directory"))?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("private")
    ));
    {
        let mut file = os::create_private(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// The canonical form of an existing path (symlinks resolved, absolute; on Windows
/// without the `\\?\` prefix when the path doesn't need it).
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    os::canonical(path)
}

/// A key for comparing paths the way the file system does (case-insensitive on Windows
/// and macOS), with `/` separators and a trailing `/`, so `starts_with` on keys means
/// "inside".
#[must_use]
pub fn path_key(path: &Path) -> String {
    let mut key = path.to_string_lossy().replace('\\', "/");
    if !key.ends_with('/') {
        key.push('/');
    }
    if case_insensitive_names() {
        key.to_lowercase()
    } else {
        key
    }
}

/// Opens `url` in the default browser (`rundll32 url.dll` on Windows, `open` on macOS,
/// `xdg-open` elsewhere). The Tauri app uses its opener plugin instead.
pub fn open_browser(url: &str) -> io::Result<()> {
    os::open_browser(url)
}

/// The OS keychain as a `keyring-core` store: Windows Credential Manager, the Secret
/// Service on Linux (D-Bus, pure Rust), the macOS Keychain. May block (D-Bus): call it
/// off the async threads.
pub fn keyring_store() -> keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> {
    os::keyring_store()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        for ts in [
            Timestamp { secs: 0, nanos: 0 },
            Timestamp {
                secs: 1_700_000_000,
                nanos: 123_456_700,
            },
            Timestamp {
                secs: -2,
                nanos: 500_000_000,
            },
            Timestamp { secs: -1, nanos: 0 },
        ] {
            let time = system_time(ts).unwrap();
            assert_eq!(timestamp(time), ts);
        }
    }

    #[test]
    fn path_keys_compare_containment() {
        let a = path_key(Path::new("/x/games"));
        let b = path_key(Path::new("/x/games/saves"));
        let c = path_key(Path::new("/x/games2"));
        assert!(b.starts_with(&a));
        assert!(!c.starts_with(&a));
    }

    #[test]
    fn private_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("token");
        write_private_file(&path, b"one").unwrap();
        write_private_file(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
