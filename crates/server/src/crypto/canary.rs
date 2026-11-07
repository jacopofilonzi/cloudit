//! Startup check that the server key is the one the database was written with.
//!
//! The setting [`CANARY_KEY`] holds base64 of a known plaintext encrypted with the key.
//! At startup every instance inserts it if missing (`ON CONFLICT DO NOTHING`, so the
//! first instance wins), reads the stored value back and decrypts it: if that fails the
//! key changed (or instances disagree) and the server refuses to start, before any
//! provider secret becomes unreadable at sign-in time.

use anyhow::bail;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sea_orm::ConnectionTrait;

use super::SecretBox;
use crate::accounts::settings;

/// Settings key of the encrypted check value.
pub const CANARY_KEY: &str = "secret_key_check";
const PLAINTEXT: &[u8] = b"cloudit secret key check v1";
const AAD: &[u8] = b"settings:secret_key_check";

/// Stores the check value if missing, then verifies the stored one (module docs).
pub async fn verify_or_store(db: &impl ConnectionTrait, secrets: &SecretBox) -> anyhow::Result<()> {
    let fresh = STANDARD.encode(secrets.encrypt(AAD, PLAINTEXT)?);
    settings::insert_if_absent(db, CANARY_KEY, &fresh).await?;
    let Some(stored) = settings::get_strict::<String>(db, CANARY_KEY).await? else {
        bail!("the secret key check value vanished from settings");
    };
    let ok = STANDARD
        .decode(stored.as_bytes())
        .ok()
        .and_then(|data| secrets.decrypt(AAD, &data).ok())
        .is_some_and(|plain| plain == PLAINTEXT);
    if !ok {
        bail!(
            "SECRET_KEY changed or wrong: the database was set up with another key (check \
             SECRET_KEY, or restore <DATA_DIR>/secret.key from the backup taken with the \
             database). Every instance must use the same key"
        );
    }
    Ok(())
}
