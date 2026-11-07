//! SeaORM entities, one per table (schema in `db::migrations`). Models only: queries
//! live with the feature that owns the table (`auth`, `devices`, `admin`…).
//!
//! `password_hash` and the `*_hash` columns hold hashes, never the secrets, since
//! models derive `Debug`. `setup_state` has no entity: its token is plaintext (the CLI
//! prints it) and is handled as a `Secret` in `setup`.

pub mod auth_providers;
pub mod device_auth_codes;
pub mod devices;
pub mod folders;
pub mod identities;
pub mod invites;
pub mod oauth_flows;
pub mod password_resets;
pub mod pending_signups;
pub mod scope_members;
pub mod scopes;
pub mod sessions;
pub mod settings;
pub mod users;
