//! The `account` table: the one signed-in account of this device.

use rusqlite::{OptionalExtension as _, params};
use serde::Serialize;

use super::Db;
use crate::error::Result;

/// Where the device token is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenLocation {
    /// The OS keychain (Windows Credential Manager, Secret Service, macOS Keychain).
    Keyring,
    /// A file readable only by the user in the data directory: the keychain was not
    /// available. The UI should warn about it.
    File,
}

impl TokenLocation {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keyring => "keyring",
            Self::File => "file",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "keyring" => Some(Self::Keyring),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

/// The signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    /// The server's base URL (`https://host[:port][/path]`, no trailing slash).
    pub server_url: String,
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub device_id: String,
    pub device_name: String,
    pub token_location: TokenLocation,
    /// RFC 3339; `None` = never expires.
    pub token_expires_at: Option<String>,
    pub signed_in_at: String,
}

impl Account {
    /// True when the token is kept in the fallback file (show a warning).
    #[must_use]
    pub fn token_warning(&self) -> bool {
        self.token_location == TokenLocation::File
    }

    /// The key identifying this account's folder links (server + user).
    #[must_use]
    pub fn link_key(&self) -> String {
        format!("{} {}", self.server_url, self.user_id)
    }
}

impl Db {
    /// The signed-in account, if any.
    pub fn account(&self) -> Result<Option<Account>> {
        self.with(|c| {
            c.query_row(
                "SELECT server_url, user_id, email, display_name, device_id, device_name,
                        token_location, token_expires_at, signed_in_at
                 FROM account WHERE id = 1",
                [],
                |r| {
                    let location: String = r.get(6)?;
                    Ok(Account {
                        server_url: r.get(0)?,
                        user_id: r.get(1)?,
                        email: r.get(2)?,
                        display_name: r.get(3)?,
                        device_id: r.get(4)?,
                        device_name: r.get(5)?,
                        token_location: TokenLocation::parse(&location)
                            .unwrap_or(TokenLocation::Keyring),
                        token_expires_at: r.get(7)?,
                        signed_in_at: r.get(8)?,
                    })
                },
            )
            .optional()
        })
    }

    /// Stores the signed-in account (replacing any previous one).
    pub fn set_account(&self, account: &Account) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO account (id, server_url, user_id, email, display_name, device_id,
                                      device_name, token_location, token_expires_at, signed_in_at)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (id) DO UPDATE SET
                    server_url = excluded.server_url, user_id = excluded.user_id,
                    email = excluded.email, display_name = excluded.display_name,
                    device_id = excluded.device_id, device_name = excluded.device_name,
                    token_location = excluded.token_location,
                    token_expires_at = excluded.token_expires_at,
                    signed_in_at = excluded.signed_in_at",
                params![
                    account.server_url,
                    account.user_id,
                    account.email,
                    account.display_name,
                    account.device_id,
                    account.device_name,
                    account.token_location.as_str(),
                    account.token_expires_at,
                    account.signed_in_at,
                ],
            )
            .map(drop)
        })
    }

    /// Updates the cached profile fields.
    pub fn update_profile(&self, email: &str, display_name: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE account SET email = ?1, display_name = ?2 WHERE id = 1",
                params![email, display_name],
            )
            .map(drop)
        })
    }

    /// Forgets the account.
    pub fn clear_account(&self) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM account", []).map(drop))
    }
}
