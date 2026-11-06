//! `cloudit-core`: the shared sync engine used by both the server and the desktop client.
//!
//! It owns everything both sides must agree on byte-for-byte:
//!
//! - `protocol`: the HTTP API request/response types. They are the single source of
//!   truth for the TypeScript models too: `make types` exports them through `ts-rs`
//!   into `packages/api-types/src/generated/`.
//! - (planned) content-defined chunking (`FastCDC`), chunk hashing (SHA-256 of the
//!   plaintext) and the content-addressed folder manifest.
//!
//! The crate does no I/O on its own: callers bring the storage and the network.

pub mod protocol;
