//! Password hashing: argon2id with the OWASP minimum (m = 19 MiB, t = 2, p = 1), PHC
//! strings in `users.password_hash`. Hashing is CPU-heavy, so it runs on the blocking
//! pool.
//!
//! [`verify_or_dummy`] always does one verification, against a fixed dummy hash when
//! the account doesn't exist or has no password, so response time doesn't reveal which
//! emails have accounts.

use std::sync::OnceLock;

use anyhow::Context as _;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::errors::ApiError;

/// Accepted password length in bytes (UTF-8).
pub const MIN_LEN: usize = 8;
pub const MAX_LEN: usize = 1024;

fn hasher() -> anyhow::Result<Argon2<'static>> {
    let params = Params::new(
        Params::DEFAULT_M_COST,
        Params::DEFAULT_T_COST,
        Params::DEFAULT_P_COST,
        None,
    )
    .map_err(|e| anyhow::anyhow!("argon2 parameters: {e}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Checks the length rules; `field` names the request field in the error.
pub fn validate(password: &str, field: &'static str) -> Result<(), ApiError> {
    if password.len() < MIN_LEN {
        return Err(ApiError::validation(
            field,
            format!("at least {MIN_LEN} bytes"),
        ));
    }
    if password.len() > MAX_LEN {
        return Err(ApiError::validation(
            field,
            format!("at most {MAX_LEN} bytes"),
        ));
    }
    Ok(())
}

fn hash_blocking(password: &[u8]) -> anyhow::Result<String> {
    let hash = hasher()?
        .hash_password(password)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("hashing a password")?;
    Ok(hash.to_string())
}

/// Hashes a password (validate it first).
pub async fn hash(password: &str) -> anyhow::Result<String> {
    let password = password.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .context("password hashing task")?
}

/// A valid hash of a random password, verified against when there is no real hash.
fn dummy_hash() -> Option<&'static str> {
    static DUMMY: OnceLock<Option<String>> = OnceLock::new();
    DUMMY
        .get_or_init(|| {
            let random = super::tokens::new_token().ok()?;
            hash_blocking(random.expose().as_bytes()).ok()
        })
        .as_deref()
}

/// Verifies `password` against `stored`; with `None` verifies against a dummy hash
/// (same cost) and returns `false`. A malformed stored hash counts as a mismatch
/// (logged).
pub async fn verify_or_dummy(password: &str, stored: Option<&str>) -> anyhow::Result<bool> {
    let password = password.as_bytes().to_vec();
    let stored = stored.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let (hash, real) = match &stored {
            Some(hash) => (hash.as_str(), true),
            None => match dummy_hash() {
                Some(hash) => (hash, false),
                None => return Ok(false),
            },
        };
        let hasher = hasher()?;
        match hasher.verify_password(&password, hash) {
            Ok(()) => Ok(real),
            Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
            Err(err) => {
                tracing::warn!(error = %err, "unreadable password hash");
                Ok(false)
            }
        }
    })
    .await
    .context("password verification task")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_and_verify() {
        let hash = hash("correct horse").await.unwrap();
        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{hash}"
        );
        assert!(verify_or_dummy("correct horse", Some(&hash)).await.unwrap());
        assert!(!verify_or_dummy("wrong horse", Some(&hash)).await.unwrap());
        assert!(!verify_or_dummy("correct horse", None).await.unwrap());
        assert!(!verify_or_dummy("x", Some("not a hash")).await.unwrap());
    }

    #[test]
    fn length_rules() {
        assert!(validate("1234567", "password").is_err());
        assert!(validate("12345678", "password").is_ok());
        assert!(validate(&"x".repeat(1024), "password").is_ok());
        assert!(validate(&"x".repeat(1025), "password").is_err());
    }
}
