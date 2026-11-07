# Project structure

cloudit is a self-hosted folder sync service: a Rust HTTP server stores versioned snapshots of
users' folders; a desktop client (planned) syncs local folders with it, and a web UI (served by the
server as a static SPA) manages accounts, scopes and folders. The UIs talk to the server only
through its JSON API under `/api/v1`, whose TypeScript types are generated from the Rust ones.

Current state (milestone 2): the server exposes health probes and serves the web SPA; the web page
shows the server status. `cloudit-core` implements the sync formats (chunking, chunk encoding,
manifest, path rules, diff; see "Sync formats"), not used by the server yet. Everything under
"Planned design" is not implemented yet.

```text
repo/
├── crates/
│   ├── core/             Rust lib `cloudit-core` — API types, chunking, chunk codec, manifest, diff (no I/O)
│   └── server/           Rust bin `cloudit-server` — axum HTTP server
├── apps/
│   └── web/              SvelteKit 3 + Svelte 5 + Tailwind 4 SPA (adapter-static)
├── packages/
│   ├── api-types/        `@cloudit/api-types` — TS types generated from Rust by ts-rs
│   └── ui/               `@cloudit/ui` — shared Svelte components, theme tokens, i18n EN/IT
├── scripts/              node scripts used by the Makefile (dev, types, set-version, help, clean)
├── .cargo/config.toml    ts-rs env (`TS_RS_EXPORT_DIR`, `TS_RS_LARGE_INT`)
├── .github/workflows/    ci.yml (make check, ubuntu + windows), release.yml (ghcr image on v*.*.* tags)
├── Cargo.toml            Cargo workspace (edition 2024, shared version, clippy lints)
├── package.json          pnpm workspace root (lint/format tooling, `packageManager`)
├── Dockerfile            stages: web (node, pnpm build) → server (rust, release build) → runtime (debian trixie-slim)
├── docker/entrypoint.sh  runs as root: chowns DATA_DIR to PUID:PGID, then setpriv → cloudit-server
├── docker-compose.yml    single instance, `./data:/data`, port 8080
├── .env.example          every supported env var, documented; `.env` is git-ignored
└── Makefile              dev commands (run `make help`)
```

Tooling: `prettier.config.js` + `.prettierignore`, `eslint.config.js` (flat config, typescript-eslint,
eslint-plugin-svelte), `clippy.toml` (unwrap/expect allowed in tests only), `rustfmt.toml`.

## Core (`crates/core/src`)

```text
crates/core/src/
├── lib.rs            crate docs: shared sync engine, no I/O
├── protocol.rs       API types: HealthResponse, ErrorCode, ApiErrorBody (ts-rs export under cfg(test))
├── error.rs          HashParseError, PathError, ChunkerError, ChunkError, ManifestError (thiserror;
│                     variants are stable kinds, `code()` gives a stable snake_case string)
├── hash.rs           ChunkHash, ManifestHash: SHA-256 newtypes, strict lowercase-hex FromStr/Display/serde
├── path.rs           validate_path: portable relative path rules (security-critical)
├── chunk/
│   ├── mod.rs        MIN/AVG/MAX_CHUNK_SIZE
│   ├── chunker.rs    FastCDC: chunk_reader (streaming, any Read), chunk_slice; golden vector
│   └── codec.rs      encode_chunk / decode_chunk (header byte + raw|zstd, bounded, hash-verified)
├── manifest/
│   ├── mod.rs        re-exports
│   ├── model.rs      Manifest, ManifestEntry, EntryKind, Timestamp, ChunkRef (JSON schema; ts-rs)
│   ├── codec.rs      canonical JSON, hash, encode (zstd), decode_manifest (bounded); golden vector
│   ├── validate.rs   Manifest::validate (rules in "Sync formats")
│   └── diff.rs       diff → ManifestDiff; chunk_hashes, missing_chunks, logical_size, unique_chunk_size
└── testutil.rs       tests only: SplitMix64 pseudo-random bytes for golden vectors
```

Each module's top-level doc comment is the normative spec of its format; "Sync formats" below
summarizes them.

## Server (`crates/server/src`)

```text
crates/server/src/
├── main.rs           wiring: config → logging → router → serve with graceful shutdown;
│                     `cloudit-server healthcheck` runs the probe instead
├── lib.rs            module list (library so tests can build the router)
├── config/
│   ├── mod.rs        Config read once from env (`from_env` / testable `from_lookup`)
│   └── secret.rs     `Secret`: redacted string (Debug/Display print `***`, no Serialize)
├── errors/mod.rs     ApiError → JSON `{code, message}` with HTTP status; internal causes logged, not returned
├── routes/
│   ├── mod.rs        router: probes, `/api/v1`, JSON 404 under `/api`, SPA fallback from WEB_DIR; AppState
│   ├── health.rs     /health (liveness), /ready (readiness)
│   └── tests.rs      router tests (tower `oneshot`)
├── logging.rs        tracing subscriber (env filter; pretty or json)
├── shutdown.rs       Ctrl+C / SIGTERM future (the only OS-specific code)
└── healthcheck.rs    HTTP/1.0 GET /health on 127.0.0.1:$PORT, std only (Docker HEALTHCHECK)
```

### API

Probes at the root (not under the base path):

- `GET /health` → `HealthResponse {status: "ok", version}`: liveness, never checks dependencies.
- `GET /ready` → `HealthResponse`: readiness; always ok for now, will check DB/storage (503
  `unavailable`).

All under `/api/v1`:

- `GET /api/v1/health` → `HealthResponse`.

Any other path under `/api` → 404 `{"code": "not_found", "message": …}`. Every other path is served
from `WEB_DIR` with `index.html` fallback (or a JSON 404 if `WEB_DIR` is missing).

Errors: JSON `ApiErrorBody {code, message}`. `code` is an `ErrorCode`: `bad_request` (400),
`not_found` (404), `unavailable` (503), `internal` (500, generic message). `message` is English, for
logs only.

### Data and state

Nothing is stored yet. `DATA_DIR` is reserved for the SQLite database and filesystem blobs (see
"Planned design").

### Configuration

Read once at startup (`config/mod.rs`); empty values count as unset. Documented in `.env.example`.

| Env var | Default | Meaning |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0` | IP address to listen on |
| `PORT` | `8080` | HTTP port |
| `PUBLIC_URL` | unset | External base URL (`http(s)://…`, trailing `/` trimmed); for OAuth redirects and links later |
| `DATA_DIR` | `./data` | Durable data directory (Docker: `/data`) |
| `WEB_DIR` | `./apps/web/build` | Built web UI served as SPA (Docker: `/app/web`) |
| `LOG_LEVEL` | `info` | Log level, used when `RUST_LOG` is unset |
| `RUST_LOG` | unset | Full tracing filter; wins over `LOG_LEVEL` |
| `LOG_FORMAT` | `pretty` | `pretty` or `json` (Docker: `json`) |
| `PUID` / `PGID` | `10001` | Docker only, read by the entrypoint (not the server): user/group the server runs as and owner of `DATA_DIR` |

## Frontend

The UI design (palette, layout, components) is not decided: it will come from user-approved
mockups in milestone 5. The current page and theme are neutral placeholders.

### Web app (`apps/web/src`)

SvelteKit 3 SPA: `ssr = false`, adapter-static with `fallback: 'index.html'`; config lives in
`apps/web/vite.config.ts` (no `svelte.config.js` in Kit 3). The Vite dev server proxies `/api`,
`/health`, `/ready` to `http://localhost:8080`. `#lib/*` maps to `src/lib/*` (package.json `imports`).

| File | Responsibility |
| --- | --- |
| `app.html`, `app.css` | HTML shell; global CSS (`tailwindcss` + `@cloudit/ui/styles.css`) |
| `routes/+layout.ts` | `ssr = false`, `prerender = false` |
| `routes/+layout.svelte` | header with app name and language switcher; sets `<html lang>` |
| `routes/+page.svelte` | server status card (calls `getHealth`) |
| `lib/api.ts` | the only API module: `APP_ROOT`/`API_BASE` from `resolve('/')`, fetch + JSON, `ApiError` with `code` |
| `lib/errors.ts` | exhaustive map from error code to i18n key |
| `lib/lang.ts` | initial language (localStorage, else browser) and persisted switching |

### Shared UI (`packages/ui/src`)

Consumed as source (no build step); `.svelte` files are compiled by the app's Vite.

| File | Responsibility |
| --- | --- |
| `styles.css` | semantic colour tokens (light/dark via `prefers-color-scheme`, placeholder values), `@utility` `card`, `btn`, `btn-primary`, `btn-secondary`, `btn-danger`; `@source` for the components |
| `lib/Button.svelte` | button with `variant` (`primary` / `secondary` / `danger`) |
| `lib/i18n/en.ts` | English dictionary; defines `MessageKey` and `Dictionary` |
| `lib/i18n/it.ts` | Italian dictionary, typed `Dictionary` (missing/extra key = type error) |
| `lib/i18n/i18n.svelte.ts` | `t(key, params)`, `getLang`/`setLang` (`$state` rune), `detectLang`, `DEFAULT_LANG = 'en'` |
| `lib/i18n/i18n.svelte.test.ts` | vitest tests for the helper |

Exports: `@cloudit/ui` (components), `@cloudit/ui/i18n`, `@cloudit/ui/styles.css`.

### API types (`packages/api-types/src`)

`generated/*.ts` are written by `make types` (`scripts/types.mjs`: clears the dir, runs the ts-rs
`export_bindings` tests, writes `generated/index.ts`). Committed; `make check` fails if stale.
`index.ts` re-exports them. Never edit by hand.

## Sync formats (`cloudit-core`)

**Durable formats**: chunks and manifests are stored on servers, so everything in this section is
pinned by golden-vector tests (`chunk::chunker`, `manifest::codec`, `hash`). Changing any of it
needs a new manifest/encoding version; never edit a golden test to make a change pass.

### Chunking

- FastCDC 2020 (crate `fastcdc` 5, module `v2020`), normalization level 1, gear seed 0.
- min 64 KiB, avg 256 KiB, max 1 MiB. A file of at most 64 KiB is one chunk; an empty file has
  no chunks; every chunk except the last is at least min, every chunk at most max.
- Streaming over any `std::io::Read` with one 1 MiB buffer (`chunk_reader`); `chunk_slice` gives
  the same boundaries for in-memory data. `Interrupted` reads are retried.

### Hashes

- `ChunkHash` = SHA-256 of the uncompressed chunk bytes; `ManifestHash` = SHA-256 of the
  canonical uncompressed manifest JSON.
- Text form: exactly 64 lowercase hex characters; anything else (uppercase, prefix, whitespace,
  other length) is rejected. TS type: `string`.

### Stored chunk encoding

`[header u8][payload]`: `0x00` raw bytes, `0x01` one zstd frame (level 3). Encode uses zstd only
if strictly smaller than raw and refuses input over 1 MiB. Decode rejects a missing or unknown
header, bounds the decompressed size to 1 MiB while streaming (zip bombs), and verifies the
SHA-256 against the expected hash (`ChunkError::HashMismatch`). The compressed bytes are not part
of the chunk identity.

### Manifest

JSON, `version: 1` (shown wrapped; the canonical form has no whitespace):

```json
{"version":1,"entries":[
  {"path":"saves/slot1.sav","kind":"file","size":70000,
   "mtime":{"secs":1700000000,"nanos":123456789},"executable":false,
   "chunks":[{"hash":"<64 hex>","size":65536},{"hash":"<64 hex>","size":4464}]},
  {"path":"screenshots","kind":"dir","size":0,"mtime":{"secs":1700000000,"nanos":0},
   "executable":false,"chunks":[]}]}
```

- `kind`: `file` | `dir`; `dir` only for **empty** directories (the others are implied by
  paths). Symlinks and special files are not represented.
- `size`: u64 bytes (0 for dirs). `mtime`: `secs` (i64 since the Unix epoch) + `nanos` (u32,
  below 10^9), so both stay exact as JS numbers. `executable`: the Unix x bit (false for dirs).
  `chunks`: ordered `{hash, size}` (size u32).
- **Canonical JSON** = `serde_json::to_vec` of the structs: compact, keys in declaration order
  (as in the example), no maps, serde_json string escaping (non-ASCII written raw). The hash is
  over these bytes; decode rejects any JSON whose re-encoding differs (whitespace, key order,
  `\u` escapes), so a manifest has exactly one hash. Unknown fields are rejected.
- **Stored** as one zstd frame (level 3) of the canonical JSON. Decode bounds the decompressed
  size (`DEFAULT_MAX_MANIFEST_BYTES` = 64 MiB, caller-configurable), reads `version` first (other
  versions give `UnsupportedVersion`), then parses and validates.
- **Validation** (on encode and decode): version 1; every path valid (below); strictly sorted by
  path in UTF-8 byte order (so no duplicates); no case-insensitive collision among the paths and
  their implied parent directories (Unicode `to_lowercase`, e.g. `A/x` vs `a/y`); no entry under
  a file or under a `dir` entry; dirs have size 0, no chunks, not executable; each chunk size in
  1..=1 MiB (the minimum is not enforced) and the sizes sum to the file size; a hash has the same
  size everywhere; `nanos` below 10^9. Errors: `ManifestError`, stable `code()` (`manifest_*`).

### Paths

Relative, `/`-separated UTF-8, stored as-is: **no Unicode normalization** (NFC and NFD spellings
are different paths; case-only differences are rejected at manifest level). Rejected: empty; over
4096 bytes; a segment over 255 bytes; leading `/` or `\`, drive prefix (`C:`, `C:x`), UNC; any
backslash; empty segment (`a//b`, trailing `/`); `.` or `..`; control characters (Unicode Cc);
any of `<>:"|?*`; a segment ending in space or dot; Windows device names case-insensitively, with
or without extension (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`-`COM9`,
`LPT0`-`LPT9`, `COM¹²³`, `LPT¹²³`). Errors: `PathError` (`path_*` codes).

### Diff

`diff(old, new)` by exact path (a rename is removed + added): `added`, `removed`, `modified`
(kind, size, chunks or executable differ), `metadata_only` (only the mtime differs: nothing to
transfer), `unchanged`; each list in path order. Helpers on `Manifest`: `chunk_hashes` (sorted
set), `missing_chunks(is_available)` (sorted, deduplicated), `logical_size` (sum of file sizes),
`unique_chunk_size` (sum of the distinct chunks' uncompressed sizes).

## Commands

`make help` lists everything. Key ones: `install`, `dev` (server + Vite, `scripts/dev.mjs` stops both
together), `dev-server`, `dev-web`, `types`, `fmt`, `lint`, `test`, `check` (fmt-check, clippy -D
warnings, eslint, svelte-check, cargo test, vitest, type freshness, web build), `build`, `docker`,
`up`/`down`, `clean`, `version V=x.y.z`.

## Planned design (NOT implemented yet)

Decided design for the next milestones, kept here so they don't have to re-derive it. Move each part
into the sections above as it gets built.

Milestones: 1 scaffolding (done) · 2 core chunking/manifest (done) · 3 server DB/storage/auth/devices ·
4 scopes/folders/sharing/sync protocol/events/jobs · 5 web UI (from mockups) · 6 desktop manual
sync + conflicts · 7 desktop auto sync/tray/autostart/installers.

Planned layout additions: `apps/desktop` (Tauri 2 + Svelte 5, milestone 6; OS code in a `platform/`
module), `crates/server/src/db/migrations/` (migrations, append-only).

### Storage and deployment

- Database via SeaORM, chosen by `DATABASE_URL`: SQLite (single instance) or PostgreSQL (HA).
- Blob storage via `object_store`, chosen by `STORAGE=fs|s3`. With S3, optional presigned URLs for
  direct client upload/download (`S3_PRESIGN`, `S3_PUBLIC_ENDPOINT`), otherwise proxied through the
  server.
- Optional Redis (`REDIS_URL`): pub/sub events across instances, rate limiting, job leases. Never
  fails a request: degrades to DB lease / in-memory limits / polling.
- Server is stateless; HA = several instances + PostgreSQL + S3 (+ Redis). An HA compose file
  (Postgres, MinIO, Redis) comes in milestone 3.

### Data model

- user > scope (user-named group) > folder (synced unit) > snapshots (immutable versions).
- Scopes are personal but shareable with other users (roles `viewer` / `editor`).
- Devices are registered per user with revocable tokens (loopback PKCE flow, RFC 8252).
- Snapshot kinds: `normal`, `conflict`, `restore`. A restore creates a new head. Retention per folder
  (last N + daily for D days); pinned snapshots are never deleted. Deleted scopes/folders go to
  trash.

### Sync protocol

- Chunks (format: "Sync formats") are stored under `owners/{owner_id}/chunks/{sha256}` (dedup
  per owner only); manifests are content-addressed by their `ManifestHash`. The server validates
  every uploaded manifest with `cloudit-core` and verifies chunk hashes on upload.
- Commit = `UPDATE folders SET head = new WHERE id = ? AND head = parent` → 201, or 409 conflict.
- On conflict the client uploads its local state as a `conflict` snapshot (not head); the user
  chooses keep-server / keep-local / decide-later.
- Desktop triggers: manual, startup, quiet period, associated process exit, SSE events. Local state in
  SQLite.

### Auth

- Local email + password (argon2id), can be disabled with `AUTH_PASSWORD=false`.
- OAuth: generic OIDC, GitHub, Discord; links to an existing account or creates one.
- Registration mode `open | invite | closed` (default `invite`). Email auto-link off by default
  (`AUTH_AUTOLINK_EMAIL`).
- Admins = `ADMIN_EMAILS` ∪ manually promoted ∪ OIDC groups (`OIDC_GROUPS_CLAIM`, default `groups`;
  `OIDC_ADMIN_GROUPS`; optional `OIDC_ALLOWED_GROUPS`). Group-derived admin is recomputed at each
  OIDC login.
