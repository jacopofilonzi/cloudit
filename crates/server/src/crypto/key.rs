//! The server key ([`SecretKey`]) and where it comes from.
//!
//! 1. `SECRET_KEY`: base64 (standard or URL-safe, padding optional) of exactly 32 bytes;
//!    anything else is a configuration error. `cloudit-server gen-secret-key` prints one.
//! 2. Unset, SQLite: `<DATA_DIR>/secret.key`, created on first start with a fresh key
//!    (owner-only permissions where the OS supports it, see [`crate::platform`]) and
//!    read on later starts. Single instance only, so one file is enough.
//! 3. Unset, PostgreSQL: startup fails. Several instances may share the database and
//!    each needs the same key; a per-instance file would silently diverge.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use sea_orm::DatabaseBackend;

/// Key length in bytes.
pub const KEY_LEN: usize = 32;
/// File name of the auto-generated key in `DATA_DIR`.
pub const KEY_FILE: &str = "secret.key";

/// A 32-byte key. `Debug` prints `***`; there is no `Display` or `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey([u8; KEY_LEN]);

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl SecretKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// A fresh random key.
    pub fn generate() -> anyhow::Result<Self> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("OS random generator: {e}"))?;
        Ok(Self(bytes))
    }

    /// Parses the base64 text form. The error never contains the input.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let text = text.trim();
        let decoded = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
            .iter()
            .find_map(|engine| engine.decode(text).ok())
            .ok_or("not valid base64")?;
        let bytes: [u8; KEY_LEN] = decoded
            .try_into()
            .map_err(|_| "must decode to exactly 32 bytes (generate one with `cloudit-server gen-secret-key`)")?;
        Ok(Self(bytes))
    }

    /// The text form: standard base64 with padding (44 characters).
    #[must_use]
    pub fn encode(&self) -> String {
        STANDARD.encode(self.0)
    }
}

/// Where the key in use came from (logged at startup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Env,
    /// Read from an existing key file.
    File(PathBuf),
    /// Generated now and written to the key file.
    Generated(PathBuf),
}

/// Resolves the key (see the module docs).
pub fn resolve(
    env_key: Option<&SecretKey>,
    backend: DatabaseBackend,
    data_dir: &Path,
) -> anyhow::Result<(SecretKey, KeySource)> {
    if let Some(key) = env_key {
        return Ok((key.clone(), KeySource::Env));
    }
    if backend != DatabaseBackend::Sqlite {
        bail!(
            "SECRET_KEY is required with PostgreSQL: every instance must use the same key to \
             read the encrypted provider secrets. Generate one with `cloudit-server \
             gen-secret-key` and set it on every instance"
        );
    }
    load_or_create(&data_dir.join(KEY_FILE))
}

/// Reads the key file, or creates it with a new key if it doesn't exist.
pub fn load_or_create(path: &Path) -> anyhow::Result<(SecretKey, KeySource)> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let key = SecretKey::parse(&text).map_err(|reason| {
                anyhow::anyhow!("invalid key file {}: {reason}", path.display())
            })?;
            return Ok((key, KeySource::File(path.to_owned())));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("reading the key file {}", path.display()));
        }
    }
    let key = SecretKey::generate()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let contents = format!("{}\n", key.encode());
    match crate::platform::write_new_private_file(path, contents.as_bytes()) {
        Ok(()) => Ok((key, KeySource::Generated(path.to_owned()))),
        // Created meanwhile (another process on the same data dir): use that one.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading the key file {}", path.display()))?;
            let key = SecretKey::parse(&text).map_err(|reason| {
                anyhow::anyhow!("invalid key file {}: {reason}", path.display())
            })?;
            Ok((key, KeySource::File(path.to_owned())))
        }
        Err(err) => Err(err).with_context(|| format!("writing the key file {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_encode() {
        let key = SecretKey::from_bytes([0xab; 32]);
        let text = key.encode();
        assert_eq!(text.len(), 44);
        assert_eq!(SecretKey::parse(&text).unwrap(), key);
        assert_eq!(SecretKey::parse(&format!("  {text}\n")).unwrap(), key);
        assert_eq!(
            SecretKey::parse(&URL_SAFE_NO_PAD.encode([0xab; 32])).unwrap(),
            key
        );
        assert!(SecretKey::parse("not base64 !!").is_err());
        assert!(SecretKey::parse(&STANDARD.encode([1u8; 31])).is_err());
        assert!(SecretKey::parse(&STANDARD.encode([1u8; 33])).is_err());
        assert_eq!(format!("{key:?}"), "***");
    }

    #[test]
    fn generated_keys_differ() {
        assert_ne!(
            SecretKey::generate().unwrap(),
            SecretKey::generate().unwrap()
        );
    }

    #[test]
    fn key_file_is_created_once_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(KEY_FILE);
        let (first, source) = load_or_create(&path).unwrap();
        assert_eq!(source, KeySource::Generated(path.clone()));
        let (second, source) = load_or_create(&path).unwrap();
        assert_eq!(source, KeySource::File(path.clone()));
        assert_eq!(first, second);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&path, "garbage").unwrap();
        let err = load_or_create(&path).unwrap_err().to_string();
        assert!(err.contains("invalid key file"), "{err}");
    }

    #[test]
    fn resolve_rules() {
        let dir = tempfile::tempdir().unwrap();
        let env = SecretKey::from_bytes([3; 32]);
        let (key, source) = resolve(Some(&env), DatabaseBackend::Postgres, dir.path()).unwrap();
        assert_eq!((key, source), (env, KeySource::Env));
        let err = resolve(None, DatabaseBackend::Postgres, dir.path()).unwrap_err();
        assert!(err.to_string().contains("SECRET_KEY is required"), "{err}");
        assert!(!dir.path().join(KEY_FILE).exists());
        let (_, source) = resolve(None, DatabaseBackend::Sqlite, dir.path()).unwrap();
        assert!(matches!(source, KeySource::Generated(_)));
        assert!(dir.path().join(KEY_FILE).is_file());
    }
}
