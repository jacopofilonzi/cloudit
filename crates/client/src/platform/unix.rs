//! Linux (and later macOS) parts of [`super`].

use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn data_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library").join("Application Support"));
    }
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local").join("share")))
}

pub fn host_name() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
}

pub fn is_executable(meta: &Metadata) -> bool {
    meta.permissions().mode() & 0o111 != 0
}

pub fn set_executable(file: &File, executable: bool) -> io::Result<()> {
    let mut permissions = file.metadata()?.permissions();
    let mode = permissions.mode();
    // The x bits follow the r bits, like `chmod +x` with the usual umask.
    let new_mode = if executable {
        mode | ((mode & 0o444) >> 2)
    } else {
        mode & !0o111
    };
    if new_mode != mode {
        permissions.set_mode(new_mode);
        file.set_permissions(permissions)?;
    }
    Ok(())
}

pub fn create_private(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` applies only on creation: tighten a leftover file too.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

pub fn open_browser(url: &str) -> io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(target_os = "macos")]
pub fn keyring_store() -> keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

/// The Secret Service over D-Bus (GNOME Keyring, KWallet, KeePassXC…), pure Rust: at
/// runtime it needs a session bus and a running Secret Service provider, which some
/// environments (e.g. the Steam Deck's game mode, headless sessions) don't have; the
/// token store then falls back to a private file.
#[cfg(not(target_os = "macos"))]
pub fn keyring_store() -> keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}
