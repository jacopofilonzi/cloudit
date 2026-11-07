//! Random tokens and their stored hashes.
//!
//! A token is 32 bytes from the OS CSPRNG, base64url without padding (43 characters).
//! Device tokens carry the prefix [`DEVICE_TOKEN_PREFIX`] so they are recognizable
//! (in a config file, a secret scanner). The database stores only
//! [`hash_token`]: lowercase hex SHA-256 of the full token string. Tokens are looked
//! up by that hash (an index lookup on a value the attacker can't choose freely), so no
//! secret is compared byte by byte in application code; where a secret must be compared
//! directly (setup token, PKCE) use [`constant_time_eq`].

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

use crate::config::Secret;

/// Prefix of device tokens (`cld_…`).
pub const DEVICE_TOKEN_PREFIX: &str = "cld_";
/// Random bytes per token.
const TOKEN_BYTES: usize = 32;

/// A new random token (base64url, no padding).
pub fn new_token() -> anyhow::Result<Secret> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("reading the OS random number generator")?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes)))
}

/// A new device token: [`DEVICE_TOKEN_PREFIX`] + a random token.
pub fn new_device_token() -> anyhow::Result<Secret> {
    let token = new_token()?;
    Ok(Secret::new(format!(
        "{DEVICE_TOKEN_PREFIX}{}",
        token.expose()
    )))
}

/// The stored form of a token: lowercase hex SHA-256 of its UTF-8 bytes.
#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// base64url (no padding) of SHA-256(`verifier`): the PKCE S256 challenge.
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Compares two secrets without leaking where they differ (the length may leak).
#[must_use]
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_base64url() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_ne!(a.expose(), b.expose());
        assert_eq!(a.expose().len(), 43);
        assert!(
            a.expose()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        let device = new_device_token().unwrap();
        assert!(device.expose().starts_with("cld_"));
        assert_eq!(device.expose().len(), 47);
    }

    #[test]
    fn hashes_are_hex_sha256() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pkce_matches_rfc7636_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq("same", "same"));
        assert!(!constant_time_eq("same", "diff"));
        assert!(!constant_time_eq("same", "same-but-longer"));
    }
}
