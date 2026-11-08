//! Ordered, append-only migration scripts (see the module docs of [`super`]).

pub const MIGRATIONS: &[&str] = &[V1];

const V1: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE account (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    server_url       TEXT NOT NULL,
    user_id          TEXT NOT NULL,
    email            TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    device_id        TEXT NOT NULL,
    device_name      TEXT NOT NULL,
    token_location   TEXT NOT NULL,
    token_expires_at TEXT,
    signed_in_at     TEXT NOT NULL
);

CREATE TABLE folders (
    id                        INTEGER PRIMARY KEY,
    local_path                TEXT NOT NULL UNIQUE,
    scope_id                  TEXT NOT NULL,
    folder_id                 TEXT NOT NULL UNIQUE,
    scope_name                TEXT NOT NULL,
    folder_name               TEXT NOT NULL,
    emoji                     TEXT,
    ignore_patterns           TEXT NOT NULL DEFAULT '',
    paused                    INTEGER NOT NULL DEFAULT 0,
    base_snapshot_id          TEXT,
    base_manifest             BLOB,
    pending_conflict_id       TEXT,
    pending_conflict_manifest BLOB,
    head_cache_id             TEXT,
    head_cache_manifest       BLOB,
    last_state                TEXT,
    last_error                TEXT,
    last_sync_at              TEXT,
    created_at                TEXT NOT NULL
);

CREATE TABLE file_index (
    folder_id   INTEGER NOT NULL REFERENCES folders (id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mtime_secs  INTEGER NOT NULL,
    mtime_nanos INTEGER NOT NULL,
    executable  INTEGER NOT NULL,
    chunks      BLOB NOT NULL,
    PRIMARY KEY (folder_id, path)
) WITHOUT ROWID;

CREATE TABLE activity (
    id          INTEGER PRIMARY KEY,
    at          TEXT NOT NULL,
    folder_id   INTEGER,
    folder_name TEXT,
    action      TEXT NOT NULL,
    outcome     TEXT NOT NULL,
    files       INTEGER NOT NULL DEFAULT 0,
    bytes       INTEGER NOT NULL DEFAULT 0,
    snapshot_id TEXT,
    error_kind  TEXT,
    error       TEXT
);
";
