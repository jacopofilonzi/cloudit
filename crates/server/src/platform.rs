//! OS-specific file handling (with `shutdown`, the only place that branches on the OS).

use std::io::Write as _;
use std::path::Path;

/// Creates `path` (failing with `AlreadyExists` if it exists) readable and writable by
/// the owner only where the OS has Unix permissions (mode 0600); on Windows the file
/// inherits the directory's ACL. The contents are flushed to disk.
pub fn write_new_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}
