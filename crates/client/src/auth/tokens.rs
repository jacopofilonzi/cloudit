//! Where the device token lives: the OS keychain, or a private file as a fallback.
//!
//! [`TokenVault::save`] tries the keychain first ([`SecretBackend`], by default the OS
//! store from `platform::keyring_store`); if that fails (no Secret Service on a Linux
//! session, e.g. the Steam Deck's game mode) it writes `<data dir>/token.json`, readable
//! only by the user (0600 on Unix), and reports [`TokenLocation::File`] so the UI can
//! warn. The account row records the location, so loading asks the right place. The
//! token never goes to the state database or a log.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;
use crate::secret::Secret;
use crate::state::account::TokenLocation;

/// Keychain service name of the entries.
const SERVICE: &str = "cloudit";

/// A keychain-like secret store (injectable for tests).
pub trait SecretBackend: Send + Sync {
    /// Stores `secret` under `key`.
    fn set(&self, key: &str, secret: &str) -> std::result::Result<(), String>;
    /// The secret under `key`, `None` if there is none.
    fn get(&self, key: &str) -> std::result::Result<Option<String>, String>;
    /// Removes `key` (no error if absent).
    fn delete(&self, key: &str) -> std::result::Result<(), String>;
}

/// The OS keychain through `keyring-core` (store created on first use).
#[derive(Default)]
pub struct KeyringBackend {
    store: OnceLock<std::result::Result<Arc<keyring_core::CredentialStore>, String>>,
}

impl KeyringBackend {
    fn entry(&self, key: &str) -> std::result::Result<keyring_core::Entry, String> {
        let store = self
            .store
            .get_or_init(|| platform::keyring_store().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)?;
        store.build(SERVICE, key, None).map_err(|e| e.to_string())
    }
}

impl SecretBackend for KeyringBackend {
    fn set(&self, key: &str, secret: &str) -> std::result::Result<(), String> {
        self.entry(key)?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }

    fn get(&self, key: &str) -> std::result::Result<Option<String>, String> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn delete(&self, key: &str) -> std::result::Result<(), String> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// An in-memory backend (tests, or a caller that keeps tokens elsewhere).
#[derive(Default)]
pub struct MemoryBackend {
    entries: Mutex<HashMap<String, String>>,
}

impl SecretBackend for MemoryBackend {
    fn set(&self, key: &str, secret: &str) -> std::result::Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get(&self, key: &str) -> std::result::Result<Option<String>, String> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .get(key)
            .cloned())
    }

    fn delete(&self, key: &str) -> std::result::Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "poisoned".to_owned())?
            .remove(key);
        Ok(())
    }
}

/// The fallback file's content.
#[derive(Serialize, Deserialize)]
struct TokenFile {
    key: String,
    token: String,
}

/// The token store: keychain first, private file as a fallback.
pub struct TokenVault {
    keychain: Option<Box<dyn SecretBackend>>,
    file: PathBuf,
}

impl std::fmt::Debug for TokenVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenVault")
            .field("keychain", &self.keychain.is_some())
            .field("file", &self.file)
            .finish()
    }
}

impl TokenVault {
    /// The OS keychain, falling back to `<data_dir>/token.json`.
    #[must_use]
    pub fn system(data_dir: &Path) -> Self {
        Self::with_backend(Some(Box::new(KeyringBackend::default())), data_dir)
    }

    /// A custom keychain backend (`None`: file only), falling back to
    /// `<data_dir>/token.json`.
    #[must_use]
    pub fn with_backend(keychain: Option<Box<dyn SecretBackend>>, data_dir: &Path) -> Self {
        Self {
            keychain,
            file: data_dir.join("token.json"),
        }
    }

    /// The fallback file's path.
    #[must_use]
    pub fn file_path(&self) -> &Path {
        &self.file
    }

    /// Stores the token of `key`; where it went.
    pub fn save(&self, key: &str, token: &Secret) -> Result<TokenLocation> {
        if let Some(keychain) = &self.keychain {
            match keychain.set(key, token.expose()) {
                Ok(()) => {
                    // A stale fallback copy would outlive a later logout otherwise.
                    self.delete_file()?;
                    return Ok(TokenLocation::Keyring);
                }
                Err(error) => {
                    tracing::warn!(%error, "OS keychain unavailable, storing the token in a private file");
                }
            }
        }
        let json = serde_json::to_vec(&TokenFile {
            key: key.to_owned(),
            token: token.expose().to_owned(),
        })
        .map_err(|e| Error::TokenStore(format!("encoding the token file: {e}")))?;
        platform::write_private_file(&self.file, &json)
            .map_err(|e| Error::TokenStore(format!("writing {}: {e}", self.file.display())))?;
        Ok(TokenLocation::File)
    }

    /// The token of `key` from `location`.
    pub fn load(&self, key: &str, location: TokenLocation) -> Result<Option<Secret>> {
        match location {
            TokenLocation::Keyring => {
                let keychain = self
                    .keychain
                    .as_ref()
                    .ok_or_else(|| Error::TokenStore("no keychain configured".to_owned()))?;
                keychain
                    .get(key)
                    .map(|t| t.map(Secret::new))
                    .map_err(Error::TokenStore)
            }
            TokenLocation::File => {
                let bytes = match std::fs::read(&self.file) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => {
                        return Err(Error::TokenStore(format!(
                            "reading {}: {e}",
                            self.file.display()
                        )));
                    }
                };
                let file: TokenFile = serde_json::from_slice(&bytes)
                    .map_err(|e| Error::TokenStore(format!("token file: {e}")))?;
                Ok((file.key == key).then(|| Secret::new(file.token)))
            }
        }
    }

    /// Removes the token of `key` everywhere.
    pub fn delete(&self, key: &str) -> Result<()> {
        if let Some(keychain) = &self.keychain
            && let Err(error) = keychain.delete(key)
        {
            tracing::warn!(%error, "could not remove the token from the OS keychain");
        }
        self.delete_file()
    }

    fn delete_file(&self) -> Result<()> {
        match std::fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::TokenStore(format!(
                "removing {}: {e}",
                self.file.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken;

    impl SecretBackend for Broken {
        fn set(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            Err("no secret service".to_owned())
        }
        fn get(&self, _: &str) -> std::result::Result<Option<String>, String> {
            Err("no secret service".to_owned())
        }
        fn delete(&self, _: &str) -> std::result::Result<(), String> {
            Err("no secret service".to_owned())
        }
    }

    #[test]
    fn keychain_first() {
        let dir = tempfile::tempdir().unwrap();
        let vault = TokenVault::with_backend(Some(Box::new(MemoryBackend::default())), dir.path());
        let token = Secret::new("cld_abc");
        assert_eq!(vault.save("k", &token).unwrap(), TokenLocation::Keyring);
        assert!(!vault.file_path().exists());
        assert_eq!(
            vault.load("k", TokenLocation::Keyring).unwrap(),
            Some(token)
        );
        vault.delete("k").unwrap();
        assert_eq!(vault.load("k", TokenLocation::Keyring).unwrap(), None);
    }

    #[test]
    fn falls_back_to_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let vault = TokenVault::with_backend(Some(Box::new(Broken)), dir.path());
        let token = Secret::new("cld_xyz");
        assert_eq!(vault.save("k", &token).unwrap(), TokenLocation::File);
        assert!(vault.file_path().exists());
        assert_eq!(vault.load("k", TokenLocation::File).unwrap(), Some(token));
        // Another account's key never gets this token.
        assert_eq!(vault.load("other", TokenLocation::File).unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(vault.file_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        vault.delete("k").unwrap();
        assert!(!vault.file_path().exists());
        assert_eq!(vault.load("k", TokenLocation::File).unwrap(), None);
    }

    #[test]
    fn file_only_vault() {
        let dir = tempfile::tempdir().unwrap();
        let vault = TokenVault::with_backend(None, dir.path());
        assert_eq!(
            vault.save("k", &Secret::new("t")).unwrap(),
            TokenLocation::File
        );
        assert!(vault.load("k", TokenLocation::Keyring).is_err());
    }
}
