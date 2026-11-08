//! Scopes, members, folders and the trash: the database side (the routes call these).
//!
//! - [`access`]: **every** request on a scope or folder resolves the caller's role
//!   through [`access::ScopeAccess`] / [`access::FolderAccess`], which answer
//!   `not_found` for anything the caller can't see (no member, in the trash, unknown
//!   id) and `forbidden` only when the caller sees it but its role is too low.
//! - [`names`]: name rules and the case-insensitive key.
//! - [`store`]: the `scopes` table.
//! - [`members`]: the `scope_members` table.
//! - [`folders`]: the `folders` table.
//!
//! Roles (`ScopeRole`, ordered): `viewer` < `editor` < `admin` < `owner`. The owner is
//! `scopes.owner_id` (no member row). Only the owner renames the scope, moves it to the
//! trash or restores it, and adds, changes or removes members; any member can leave.
//! Admins move folders to the trash, restore them and change their retention; editors
//! create and rename folders. Server admins get no implicit access. A disabled,
//! pending or rejected account keeps its memberships, but authentication refuses it.
//!
//! **Names** are unique among siblings outside the trash, ignoring case; the database
//! enforces it with the `active_key` columns (migration 000007), so creating,
//! renaming and restoring are single statements and a duplicate fails with
//! `name_taken` whatever the timing.
//!
//! **Trash**: moving a scope or folder to the trash sets `deleted_at` / `deleted_by`
//! and clears `active_key` in one conditional UPDATE. A folder of a trashed scope
//! keeps its own state but is unreachable until the scope is restored. Permanent
//! deletion: the `trash_purge` job (`jobs::trash_purge`) deletes items trashed more
//! than `trash_retention_days` ago, and those marked with `purge_requested_at` (the
//! permanent delete endpoints); a marked item is no longer listed nor restorable.

pub mod access;
pub mod folders;
pub mod members;
pub mod names;
pub mod store;
