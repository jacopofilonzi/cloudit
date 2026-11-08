//! Windows parts of [`super`].

use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join("AppData").join("Local"))
        })
}

pub fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

pub fn is_executable(_meta: &Metadata) -> bool {
    false
}

pub fn set_executable(_file: &File, _executable: bool) -> io::Result<()> {
    Ok(())
}

pub fn create_private(path: &Path) -> io::Result<File> {
    // Files under %LOCALAPPDATA% inherit the user's profile ACL (the user, SYSTEM and
    // administrators): nothing to tighten.
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    // `canonicalize` returns `\\?\C:\…`; drop the prefix for plain drive paths so the
    // stored and displayed paths look normal (UNC and long paths keep it).
    let text = canonical.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        let plain_drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
            && rest.len() < 260;
        if plain_drive {
            return Ok(PathBuf::from(OsString::from(rest)));
        }
    }
    Ok(canonical)
}

pub fn open_browser(url: &str) -> io::Result<()> {
    // `rundll32 url.dll,FileProtocolHandler` takes the URL as one argument: no `cmd`
    // quoting issues with `&` in the query string.
    Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

pub fn keyring_store() -> keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}
