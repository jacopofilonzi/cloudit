//! Accounts and their database operations (the routes call these).
//!
//! - [`users`]: the `users` table.
//! - [`admin`]: admin-flag and disable changes, with the race-free last-admin guard.
//! - [`setup`]: first-run setup token (`setup_state`).
//! - [`settings`]: admin-editable server settings (`settings`).
//! - [`invites`], [`password_resets`]: single-use tokens.
//!
//! Every single-use token (setup, invite, reset, device code) is consumed with one
//! conditional UPDATE whose affected-row count decides the winner; nothing is "read,
//! check, then write". In a transaction that writes, the first statement is a write:
//! SQLite can't upgrade a read transaction to a write one while another connection
//! writes (it fails with `SQLITE_BUSY` instead of waiting).

pub mod admin;
pub mod invites;
pub mod password_resets;
pub mod settings;
pub mod setup;
pub mod users;
