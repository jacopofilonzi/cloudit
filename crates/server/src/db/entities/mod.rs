//! SeaORM entities, one per table (schema in `db::migrations`). Models only: queries
//! live with the feature that owns the table (`auth`, `devices`, `admin`…).
//!
//! `password_hash` and the `*_hash` columns hold hashes, never the secrets, since
//! models derive `Debug`. `setup_state` has no entity: its token is plaintext (the CLI
//! prints it) and is handled as a `Secret` in `setup`.

pub mod device_auth_codes;
pub mod devices;
pub mod invites;
pub mod password_resets;
pub mod sessions;
pub mod settings;
pub mod users;
