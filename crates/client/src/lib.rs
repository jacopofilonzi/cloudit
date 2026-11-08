//! `cloudit-client`: the sync engine of the cloudit desktop client (no UI).
//!
//! The desktop app (Tauri) and a future CLI are thin layers over [`Engine`]: sign in
//! through the browser, link local folders to server folders, compute their status,
//! sync them, decide conflicts, follow server events. Everything is built on the
//! formats of `cloudit-core` and the server's `/api/v1`.
//!
//! # The safety rule (main invariant)
//!
//! **The engine never overwrites or deletes a local file whose current content isn't
//! already stored on the server.** There are no local backup copies; instead:
//!
//! - a pull without local changes only replaces files equal to the last synced version
//!   (the base snapshot, on the server — checked to still exist) or to the head;
//! - a pull or a "keep the server's version" with local changes first commits the local
//!   state as a `conflict` snapshot (chunks uploaded, commit accepted); only then may
//!   local files be replaced, and if that upload fails nothing is touched;
//! - right before replacing or deleting each file, the engine re-checks it against the
//!   scan (size + mtime, and the chunk hashes when the scan found it "racy"); if it
//!   changed (a game just saved), it is left alone, the rest of the pull stops and the
//!   folder is reported as changed ([`ErrorKind::LocalChanged`]).
//!
//! Enforced in `engine::ops` (decision and conflict upload), `sync::plan`
//! ([`sync::plan::SafeSet`], [`sync::plan::unsafe_paths`]), `sync::pull` (refuses an
//! unsafe plan) and `sync::stage` (re-checks, atomic renames); tested end to end in
//! `tests/safety.rs`.
//!
//! Also: nothing is written inside a synced folder except the synced files themselves
//! and short-lived temp files (`.cloudit-tmp-*`, see [`sync::stage`]); all state lives
//! in the engine's data directory.
//!
//! # Design
//!
//! - [`state`]: local SQLite state (account, links, base snapshots, file index,
//!   activity), append-only migrations on `user_version`.
//! - [`api`]: the HTTP client (retries, error mapping, chunk transfers).
//! - [`auth`]: browser sign-in (loopback + PKCE) and token storage (OS keychain, private
//!   file fallback).
//! - [`scan`]: folder walking, ignore rules, hashing with the file index, problems.
//! - [`sync`]: three-way comparison, status, push, safe pull.
//! - [`engine`]: the façade and the orchestration (locks, conflict flow, activity).
//! - [`events`]: following server events (SSE + heads polling).
//! - [`progress`]: progress reports and cancellation of every long operation.
//! - [`platform`]: everything OS-specific.
//!
//! # Status model
//!
//! Each path is compared three ways (local scan `L`, base `B`, server head `H`; content
//! only, mtimes ignored, ignored paths excluded): `L == H` converged, else changed only
//! locally, only remotely, or on both sides. The folder is `conflict` when both sides
//! changed anything (whole-folder decision: a game's save set is one unit), else
//! `local_changes` / `remote_changes` / `in_sync` (see [`SyncState`] for the full list
//! and precedence).

pub mod api;
pub mod auth;
pub mod engine;
mod error;
pub mod events;
pub mod platform;
pub mod progress;
pub mod scan;
mod secret;
pub mod state;
pub mod sync;

pub use auth::login::PendingLogin;
pub use auth::tokens::{KeyringBackend, MemoryBackend, SecretBackend, TokenVault};
pub use engine::{Engine, EngineConfig, SyncOutcome, SyncReport};
pub use error::{Error, ErrorKind, LinkRefusal, LoginFailure, Result};
pub use progress::{CancellationToken, OpContext, Phase, Progress};
pub use secret::Secret;
pub use state::account::{Account, TokenLocation};
pub use state::activity::ActivityEntry;
pub use state::folders::LinkedFolder;
pub use sync::status::{FolderStatus, SyncState};
