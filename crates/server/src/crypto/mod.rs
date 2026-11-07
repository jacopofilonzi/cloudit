//! Encryption of secrets the server must read back (identity-provider client secrets).
//!
//! - [`key`]: the 32-byte server key: `SECRET_KEY` (base64), or, with SQLite only, an
//!   auto-generated `<DATA_DIR>/secret.key`. PostgreSQL deployments (possibly several
//!   instances) must set `SECRET_KEY`: every instance needs the same key.
//! - [`SecretBox`]: XChaCha20-Poly1305 with a random 24-byte nonce per message. Stored
//!   form: `[version u8 = 1][nonce 24 bytes][ciphertext + 16-byte tag]`. The associated
//!   data names what the value belongs to (e.g. `auth_providers:<id>:client_secret`),
//!   so a ciphertext copied to another row or setting fails to decrypt.
//! - [`canary`]: an encrypted known value in `settings`, checked at startup so a
//!   changed or wrong key stops the server instead of failing later at sign-in.
//!
//! Losing the key makes the stored client secrets unreadable (admins re-enter them);
//! back it up together with the database.

pub mod canary;
pub mod key;

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub use key::SecretKey;

/// Version byte of the stored format.
const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Decryption failed: wrong key, wrong associated data, or corrupted / unknown data.
#[derive(Debug, thiserror::Error)]
#[error("cannot decrypt: wrong key, wrong context or corrupted data")]
pub struct DecryptError;

/// Authenticated encryption with the server key (see the module docs).
#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(***)")
    }
}

impl SecretBox {
    #[must_use]
    pub fn new(key: &SecretKey) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(&(*key.bytes()).into()),
        }
    }

    /// Encrypts `plaintext` bound to `aad`.
    pub fn encrypt(&self, aad: &[u8], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("OS random generator: {e}"))?;
        let ciphertext = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypts a value produced by [`SecretBox::encrypt`] with the same `aad`.
    pub fn decrypt(&self, aad: &[u8], data: &[u8]) -> Result<Vec<u8>, DecryptError> {
        let (&version, rest) = data.split_first().ok_or(DecryptError)?;
        if version != VERSION || rest.len() < NONCE_LEN + TAG_LEN {
            return Err(DecryptError);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| DecryptError)?;
        self.cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| DecryptError)
    }

    /// [`SecretBox::decrypt`] of a UTF-8 string.
    pub fn decrypt_string(&self, aad: &[u8], data: &[u8]) -> Result<String, DecryptError> {
        String::from_utf8(self.decrypt(aad, data)?).map_err(|_| DecryptError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(byte: u8) -> SecretBox {
        SecretBox::new(&SecretKey::from_bytes([byte; 32]))
    }

    #[test]
    fn round_trip_and_binding() {
        let sb = boxed(7);
        let a = sb.encrypt(b"row:a", b"client-secret").unwrap();
        let b = sb.encrypt(b"row:a", b"client-secret").unwrap();
        assert_ne!(a, b, "random nonce per message");
        assert_eq!(a[0], VERSION);
        assert_eq!(a.len(), 1 + NONCE_LEN + b"client-secret".len() + TAG_LEN);
        assert_eq!(sb.decrypt_string(b"row:a", &a).unwrap(), "client-secret");
        // Another row, another key, tampering, truncation, unknown version.
        assert!(sb.decrypt(b"row:b", &a).is_err());
        assert!(boxed(8).decrypt(b"row:a", &a).is_err());
        let mut tampered = a.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(sb.decrypt(b"row:a", &tampered).is_err());
        assert!(sb.decrypt(b"row:a", &a[..20]).is_err());
        assert!(sb.decrypt(b"row:a", &[]).is_err());
        let mut v2 = a;
        v2[0] = 2;
        assert!(sb.decrypt(b"row:a", &v2).is_err());
    }

    #[test]
    fn debug_hides_the_key() {
        assert_eq!(format!("{:?}", boxed(1)), "SecretBox(***)");
    }
}
