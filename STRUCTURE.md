# Project structure

cloudit is a self-hosted folder sync service: a Rust HTTP server stores versioned snapshots of
users' folders; a desktop client (planned) syncs local folders with it, and a web UI (served by the
server as a static SPA) manages accounts, scopes and folders. The UIs talk to the server only
through its JSON API under `/api/v1`, whose TypeScript types are generated from the Rust ones.

Current state (milestone 3b): the server has its infrastructure (database with migrations and
leases, blob storage, optional Redis with a rate limiter, readiness checks), exposes health probes,
serves the web SPA and has the account API: first-run setup, password accounts with web sessions,
invites, password resets, desktop device login (loopback + PKCE) and admin endpoints. API only:
the web pages come with the mockups of milestone 5 (the web page shows the server status).
`cloudit-core` implements the sync formats (chunking, chunk encoding, manifest, path rules, diff;
see "Sync formats"), not used by the server yet. Everything under "Planned design" is not
implemented yet.

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
├── scripts/              node scripts used by the Makefile (dev, types, set-version, help, clean, test-ha)
├── .cargo/config.toml    ts-rs env (`TS_RS_EXPORT_DIR`, `TS_RS_LARGE_INT`)
├── .github/workflows/    ci.yml (make check on ubuntu + windows; server tests with PostgreSQL/Garage/Valkey),
│                         release.yml (ghcr image on v*.*.* tags)
├── Cargo.toml            Cargo workspace (edition 2024, shared version, clippy lints)
├── package.json          pnpm workspace root (lint/format tooling, `packageManager`)
├── Dockerfile            stages: web (node, pnpm build) → server (rust, release build) → runtime (debian trixie-slim)
├── docker/entrypoint.sh  runs as root: chowns DATA_DIR to PUID:PGID, then setpriv → cloudit-server
├── docker/Caddyfile      HA load balancer: round robin over the replicas, health check on /ready
├── docker-compose.yml    single instance, `./data:/data`, port 8080 (SQLite + fs blobs)
├── docker/garage.toml    Garage config for the HA example, test-ha and CI (example secrets, dev only)
├── docker-compose.ha.yml HA example: 2 replicas + Caddy (:8080), postgres:18, Garage
│                         (dxflrs/garage, single node, creates its bucket), valkey:9; also the
│                         services of `make test-ha`
├── .env.example          every supported env var, documented; `.env` is git-ignored
└── Makefile              dev commands (run `make help`)
```

Tooling: `prettier.config.js` + `.prettierignore`, `eslint.config.js` (flat config, typescript-eslint,
eslint-plugin-svelte), `clippy.toml` (unwrap/expect allowed in tests only), `rustfmt.toml`.

## Core (`crates/core/src`)

```text
crates/core/src/
├── lib.rs            crate docs: shared sync engine, no I/O
├── protocol/         API types (ts-rs export under cfg(test)); conventions in mod.rs docs
│   ├── mod.rs        HealthResponse, ReadyResponse (+ ReadyStatus, ReadyComponents,
│   │                 ComponentStatus), ErrorCode, ApiErrorBody {code, message, field?}
│   ├── sensitive.rs  Sensitive: password/token string in API bodies, Debug prints `***`
│   ├── auth.rs       RegistrationMode, AuthConfig, AuthProvider, SetupStatus, SetupRequest,
│   │                 RegisterRequest, LoginRequest, PasswordResetRequest
│   ├── me.rs         UserProfile, UpdateMeRequest, ChangePasswordRequest, SessionInfo
│   ├── devices.rs    TokenTtl (1h/1d/7d/30d/1y), CodeChallengeMethod, DeviceAuthorize*,
│   │                 DeviceToken*, DeviceInfo, UpdateDeviceRequest
│   └── admin.rs      AdminUser, AdminUserList, AdminUserUpdate, PasswordResetLink,
│                     CreateInviteRequest, InviteLink, InviteStatus, InviteInfo, ServerSettings,
│                     UpdateSettingsRequest
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
├── main.rs           subcommand (cli::parse) or wiring: config → logging → AppState::init →
│                     router → serve (with peer addresses) with graceful shutdown
├── cli.rs            subcommands: setup-token, admin reset-password <email>, healthcheck, help
├── lib.rs            module list (library so tests can build the router)
├── state.rs          AppState: config, db, BlobStore, Option<Redis>, RateLimiter, instance_id,
│                     Leases, setup_done; `init` connects + migrates + setup::init (fatal on
│                     error); Redis is never fatal
├── validate.rs       email (trim + lowercase + shape), text / display_name (trimmed, bounded)
├── time.rs           now (µs precision), add, rfc3339 (API timestamps `2026-01-31T12:00:00Z`)
├── accounts/         account data (queries the routes call); single-use token rules in mod.rs
│   ├── users.rs      insert (email_taken on unique violation), find, find_by_email,
│   │                 set_password, update_profile, list + count
│   ├── admin.rs      update_flags (admin / disabled) with the race-free last-admin guard
│   ├── setup.rs      setup_state: init at startup (same token on every instance), status,
│   │                 check_token, complete (conditional UPDATE)
│   ├── settings.rs   settings table: registration_mode (default invite)
│   ├── invites.rs    create, find_usable, consume (conditional UPDATE), list, revoke, status
│   └── password_resets.rs  create (24 h), consume (conditional UPDATE)
├── auth/             authentication (the module docs list the pieces)
│   ├── tokens.rs     new_token (32 random bytes, base64url), cld_ device tokens, hash_token
│   │                 (SHA-256 hex), pkce_challenge, constant_time_eq
│   ├── password.rs   argon2id (m=19 MiB, t=2, p=1) on the blocking pool; verify_or_dummy
│   ├── sessions.rs   cloudit_session cookie; create, authenticate, slide_if_due, list, delete
│   ├── client.rs     ClientInfo: client IP, https, own origin, user agent
│   ├── csrf.rs       Origin / Sec-Fetch-Site check
│   ├── middleware.rs /api/v1 layer: setup gate, CSRF, Auth (bearer or cookie), cookie refresh
│   ├── extractors.rs SessionUser, AnyUser, AdminUser, Client
│   └── limits.rs     rate limits of the auth endpoints + enforce
├── devices/
│   ├── mod.rs        devices table: create, authenticate (bearer), touch, list, find, rename,
│   │                 revoke
│   ├── codes.rs      device_auth_codes: create (2 min), consume (conditional UPDATE)
│   └── redirect.rs   loopback redirect_uri validation, redirect URL with code + state
├── config/
│   ├── mod.rs        Config read once from env (`from_env` / testable `from_lookup`)
│   ├── env.rs        typed getters (empty = unset), bool and duration (`900`, `15m`, `1h`, `1d`)
│   ├── storage.rs    StorageConfig (`Fs { dir }` | `S3(S3Config)`), validation of the S3_* variables
│   └── secret.rs     `Secret`: redacted string (Debug/Display print `***`, no Serialize)
├── db/
│   ├── mod.rs        connect (SQLite pragmas, pool sizes), ping; table conventions in the module docs
│   ├── migrate.rs    startup migrations: SQLite VACUUM INTO backup, PostgreSQL advisory lock
│   ├── migrations/   Migrator + one file per migration (append-only, portable schema builder):
│   │                 000001 leases, 000002 accounts (users, sessions, setup_state, settings),
│   │                 000003 tokens (invites, password_resets, devices, device_auth_codes)
│   ├── entities/     SeaORM 2 models (`#[sea_orm::model]`, no relations), one file per table
│   │                 (none for setup_state: its plaintext token is read as a Secret)
│   ├── lease.rs      Leases: try_acquire / renew / release, one atomic statement each
│   └── testing.rs    tests only: TestDb (SQLite temp file, or a fresh database on TEST_DATABASE_URL)
├── storage/
│   ├── mod.rs        BlobStore over object_store: put, get (stream), get_bytes, head, delete, list,
│   │                 presign_put / presign_get, check (readiness)
│   └── tests.rs      fs round trip, offline presign checks, live S3 test (TEST_S3_*)
├── cache/
│   ├── mod.rs        Redis: lazy ConnectionManager, `run` with 2 s timeout + 5 s backoff, rate-limited warnings
│   └── rate_limit.rs RateLimiter: fixed window in Redis (shared) or in memory (fallback)
├── errors/mod.rs     ApiError → JSON `{code, message, field?}` with HTTP status (+ Retry-After);
│                     internal causes logged, not returned
├── routes/
│   ├── mod.rs        router: probes, `/api/v1` (behind auth::middleware), JSON 404 under
│   │                 `/api`, SPA fallback from WEB_DIR
│   ├── extract.rs    ApiJson (rejections as bad_request), parse_id, model → API conversions
│   ├── health.rs     /health (liveness), /ready (database + storage checks, Redis reported)
│   ├── setup.rs      /setup
│   ├── auth.rs       /auth/* (config, register, login, logout, password-reset)
│   ├── me.rs         /me, /me/password, /me/sessions
│   ├── devices.rs    /devices (authorize, token, list, rename, revoke)
│   ├── admin.rs      /admin/* (users, password resets, invites, settings)
│   └── tests/        router tests (tower `oneshot`, real AppState; PostgreSQL with
│                     TEST_DATABASE_URL): mod.rs (probes, SPA), support.rs (TestApp harness),
│                     setup, auth, me, devices, admin
├── logging.rs        tracing subscriber (env filter; pretty or json)
├── shutdown.rs       Ctrl+C / SIGTERM future (the only OS-specific code)
└── healthcheck.rs    HTTP/1.0 GET /health on 127.0.0.1:$PORT, std only (Docker HEALTHCHECK)
```

### API

Probes at the root (not under the base path):

- `GET /health` → `HealthResponse {status: "ok", version}`: liveness, never checks dependencies.
- `GET /ready` → `ReadyResponse {status: "ok" | "unavailable", version, components: {database,
  storage, redis}}`, each component `ok` / `unavailable` / `disabled`. 200 only if the database
  (`SELECT 1`) and storage (fs: root writable; s3: bucket listable) are `ok`, else 503 with the
  same body. Redis is reported but never makes it fail. Each check has a 3 s timeout.

All under `/api/v1` (types in `cloudit-core::protocol`). Auth column: **none**; **session** (web
session cookie only: a device token gets 403 `forbidden`); **any** (session or `Authorization:
Bearer cld_…`); **admin** (session of an effective admin).

| Method and path | Auth | Body → answer |
| --- | --- | --- |
| `GET /health` | none | `HealthResponse` |
| `GET /setup` | none | `SetupStatus {required}` |
| `POST /setup` | none, rate limited | `SetupRequest` → 201 `UserProfile` + cookie (first admin, email verified) |
| `GET /auth/config` | none | `AuthConfig {setup_required, registration_mode, providers: []}` |
| `POST /auth/register` | none, rate limited | `RegisterRequest` → 201 `UserProfile` + cookie |
| `POST /auth/login` | none, rate limited | `LoginRequest` → 200 `UserProfile` + cookie |
| `POST /auth/logout` | none | → 204; deletes the current session, clears the cookie |
| `POST /auth/password-reset` | none, rate limited | `PasswordResetRequest` → 204 (no sign-in) |
| `GET /me` | any | `UserProfile` |
| `PATCH /me` | session | `UpdateMeRequest` → `UserProfile` |
| `POST /me/password` | session | `ChangePasswordRequest` → 204; other sessions signed out |
| `GET /me/sessions` | session | `SessionInfo[]` (current one marked) |
| `DELETE /me/sessions/{id}` | session | → 204 |
| `POST /devices/authorize` | session | `DeviceAuthorizeRequest` → `{redirect_url}` |
| `POST /devices/token` | none, rate limited, no CSRF check | `DeviceTokenRequest` → `DeviceTokenResponse` |
| `GET /devices` | session | `DeviceInfo[]` (revoked and expired included) |
| `PATCH /devices/{id}` | session | `UpdateDeviceRequest` → `DeviceInfo` |
| `DELETE /devices/{id}` | session | → 204 (revoke, soft) |
| `GET /admin/users?limit=&offset=` | admin | `AdminUserList` (oldest first; limit 1..=200, default 50) |
| `PATCH /admin/users/{id}` | admin | `AdminUserUpdate {admin?, disabled?}` → `AdminUser` |
| `POST /admin/users/{id}/password-reset` | admin | → 201 `PasswordResetLink {token, path: "/reset-password/<token>", expires_at}` |
| `GET /admin/invites` | admin | `InviteInfo[]` (newest first, no tokens) |
| `POST /admin/invites` | admin | `CreateInviteRequest {email?, expires_in?}` (default 7d) → 201 `InviteLink {id, token, path: "/invite/<token>", expires_at}` |
| `DELETE /admin/invites/{id}` | admin | → 204 (revoke, soft) |
| `GET /admin/settings` | admin | `ServerSettings {registration_mode}` |
| `PATCH /admin/settings` | admin | `UpdateSettingsRequest` → `ServerSettings` |

Any other path under `/api` → 404 `{"code": "not_found", "message": …}`. Every other path is served
from `WEB_DIR` with `index.html` fallback (or a JSON 404 if `WEB_DIR` is missing).

Errors: JSON `ApiErrorBody {code, message, field?}`; `message` is English, for logs only; `field`
names the request field with `validation_failed`. Codes (`ErrorCode`) and statuses: `bad_request`
400 (malformed JSON, unknown fields, bad query), `not_found` 404, `unavailable` 503, `internal` 500
(generic message), `unauthorized` 401, `forbidden` 403, `invalid_credentials` 401 (wrong email or
password: same answer), `account_disabled` 403, `registration_closed` 403, `invite_invalid` 400,
`token_invalid` 400 (reset token, device code, PKCE verifier, redirect mismatch), `email_taken`
409, `validation_failed` 422, `rate_limited` 429 (`Retry-After` in seconds), `csrf_failed` 403,
`setup_required` 409, `setup_already_done` 409, `setup_token_invalid` 403, `last_admin` 409.

### Authentication

- **First-run setup**: at startup, if there is no user, every instance runs `INSERT INTO
  setup_state (1, <random token>, now) ON CONFLICT (id) DO NOTHING`, reads the row back and logs
  the token at WARN while setup is pending, so all instances print the same one (`SETUP_TOKEN`
  replaces it while pending). `POST /setup` consumes it in one transaction: `UPDATE setup_state
  SET token = NULL, completed_at = now WHERE id = 1 AND completed_at IS NULL AND token = ?`
  (exactly one winner), then creates the admin and stores `registration_mode`. The row is never
  deleted. While pending, every `/api/v1` route except `/health`, `/auth/config` and `/setup`
  answers `setup_required` (409: a server state, not an instance failure); once an instance has
  seen setup completed it caches that. `cloudit-server setup-token` prints the pending token.
- **Passwords**: argon2id m=19 MiB t=2 p=1 (PHC strings), 8..=1024 bytes. Login always runs one
  verification (against a dummy hash for unknown emails or password-less accounts).
  `password_hash` is nullable (identity-provider accounts, 3c).
- **Sessions**: cookie `cloudit_session` = 32 random bytes base64url; `HttpOnly; SameSite=Lax;
  Path=/; Max-Age` 30 days; `Secure` when `PUBLIC_URL` is https or a trusted proxy sends
  `X-Forwarded-Proto: https`. Sliding: a session expires 30 days after its last recorded use;
  `last_seen_at` / `expires_at` are written at most once per hour (the cookie is refreshed then).
  Logout deletes the row. A password change signs out the other sessions, a password reset all of
  them (devices stay).
- **Devices** (RFC 8252 loopback + PKCE S256): `authorize` (session) checks the challenge (43
  base64url characters), the redirect URI (`http://127.0.0.1:<port>/…` or `http://[::1]:<port>/…`,
  explicit port, no query, fragment or `localhost`), `state` (1..=512 characters), name, os and
  `token_ttl` (1h, 1d, 7d, 30d default, 1y), and stores a 2-minute code. `token` consumes the code
  first (conditional UPDATE), then checks the redirect URI (exact match) and SHA-256(verifier) in
  constant time: any failure burns the code. The token `cld_<base64url>` has a fixed expiry;
  revoking sets `revoked_at`; `last_seen_at` is written at most every 5 minutes.
- **Disabled accounts**: login refused (after the password check); sessions and device tokens
  answer `account_disabled` at once (checked on every request). Effective admin = `admin ||
  admin_from_groups`. `PATCH /admin/users` refuses to leave no enabled effective admin
  (`last_admin`); concurrent changes are serialized by a write on the `setup_state` row as the
  first statement of the transaction (`accounts/admin.rs`).
- **CSRF**: every unsafe request (not GET/HEAD/OPTIONS) without a bearer token, except
  `POST /devices/token`, needs an `Origin` equal to the server's origin (`PUBLIC_URL`'s if set,
  else `http(s)://` + `Host`, or `X-Forwarded-Host` when trusted) or, without `Origin`,
  `Sec-Fetch-Site: same-origin`; else `csrf_failed`. Login, register and setup included (login
  CSRF). No CORS.
- **Client IP** (rate limits): the TCP peer; with `TRUST_PROXY_HEADERS=true` the rightmost
  `X-Forwarded-For` entry that is not loopback, private or link-local (the rightmost entry if all
  are internal).
- **Rate limits** (fixed windows, so up to twice the limit may pass across a boundary; shared
  through Redis when available): login 20 / 5 min per IP and 10 / 15 min per email; register
  10 / h per IP; setup 10 / 15 min per IP; device token 30 / 15 min per IP; password reset
  10 / 15 min per IP; current-password checks (password or email change) 10 / 15 min per user.
- **Secrets**: tokens are stored only as SHA-256 hex (`*_hash` columns) and never logged; API
  bodies carry them as `Sensitive`. The setup token is the one secret stored in plaintext (the
  CLI prints it) and logged on purpose while setup is pending.

### Data and state

- **Database** (SeaORM 2, `DATABASE_URL`): SQLite `<DATA_DIR>/cloudit.db` by default (WAL,
  `foreign_keys=ON`, `synchronous=NORMAL`, 5 s busy timeout), or PostgreSQL. Conventions for
  every table (module docs of `db`): UUID v7 ids in `uuid` columns (native on PostgreSQL,
  16-byte BLOB on SQLite: always bind `Uuid`, never strings); UTC `chrono::DateTime<Utc>` in
  `timestamp_with_time_zone` columns (`timestamptz` / RFC 3339 text that sorts correctly);
  snake_case plural table names; each migration runs in a transaction.
- **Tables** (plus SeaORM's `seaql_migrations`):
  - `leases(name PK, holder, expires_at)`;
  - `users(id, email UNIQUE (trimmed, lowercased), email_verified, display_name, password_hash?,
    admin, admin_from_groups, disabled_at?, created_at, updated_at)`;
  - `sessions(id, token_hash UNIQUE, user_id → users, created_at, last_seen_at, expires_at,
    user_agent?)` (index on user_id; user agent truncated to 256 characters);
  - `setup_state(id = 1 PK, token?, created_at, completed_at?)`: one row, never deleted;
  - `settings(key PK, value (JSON text), updated_at, updated_by? → users)`; key
    `registration_mode` = `"open" | "invite" | "closed"` (missing = `invite`);
  - `invites(id, token_hash UNIQUE, created_by → users, email?, created_at, expires_at, used_at?,
    used_by? → users, revoked_at?)`;
  - `password_resets(id, token_hash UNIQUE, user_id → users, created_by? → users (null = CLI),
    created_at, expires_at (24 h), used_at?)`;
  - `devices(id, user_id → users, name, os, token_hash UNIQUE, created_at, last_seen_at?,
    expires_at, revoked_at?)` (index on user_id);
  - `device_auth_codes(id, code_hash UNIQUE, user_id → users, code_challenge, redirect_uri,
    device_name, os, token_ttl_secs, created_at, expires_at (2 min), used_at?)`.

  Expired sessions, codes and used tokens are not cleaned up yet (a GC job is planned).
- **Migrations** run at startup (`db/migrate.rs`). SQLite: if any are pending and the database
  isn't new, `VACUUM INTO <DATA_DIR>/backups/cloudit-<UTC YYYYMMDDTHHMMSSZ>-pre-migrate.db`
  first; a failed backup aborts startup; backups are never pruned automatically. PostgreSQL:
  under `pg_advisory_lock(0x636c6f7564697400)`, so concurrent instances migrate once; no
  automatic backup (the operator runs `pg_dump`). Unknown applied migrations (a downgrade) abort.
- **Leases** (`db/lease.rs`): `INSERT … ON CONFLICT (name) DO UPDATE … WHERE holder = me OR
  expires_at < now`, success = 1 row affected; `renew` fails once expired. Holder = the random
  per-process `instance_id`. Instance clocks must be in sync. Redis is deliberately not used for
  leases (one mechanism, always available).
- **Blobs** (`object_store` 0.14, `STORAGE`): `fs` = `LocalFileSystem` at `STORAGE_FS_DIR`
  (atomic writes, fsync on); `s3` = `AmazonS3` (path-style by default, `http` endpoints
  allowed). Keys must be canonical object paths. Presigned URLs only with `S3_PRESIGN=true`;
  when `S3_PUBLIC_ENDPOINT` is set, a second signer-only `AmazonS3` built with it signs the
  URLs (SigV4 signs the host), so they work for clients.
- **Redis** (`REDIS_URL`, optional): Redis or Valkey via the `redis` crate's lazy
  `ConnectionManager`. Used now by the rate limiter (keys `cloudit:rl:<key>:<window>`); any
  error or 2 s timeout degrades to the in-memory path and skips Redis for 5 s.

### Configuration

Read once at startup (`config/mod.rs`); empty values count as unset. Documented in `.env.example`.

| Env var | Default | Meaning |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0` | IP address to listen on |
| `PORT` | `8080` | HTTP port |
| `PUBLIC_URL` | unset | External base URL (`http(s)://…`, trailing `/` trimmed): the origin for CSRF checks, `Secure` cookie when https, full URL printed by `admin reset-password`; OAuth redirects later |
| `TRUST_PROXY_HEADERS` | `false` | Trust `X-Forwarded-For/-Proto/-Host` (client IP, https, origin); only behind a reverse proxy that sets them |
| `SETUP_TOKEN` | unset (random) | Fixed first-run setup token (secret, at least 16 characters) |
| `DATA_DIR` | `./data` | Durable data directory (Docker: `/data`) |
| `WEB_DIR` | `./apps/web/build` | Built web UI served as SPA (Docker: `/app/web`) |
| `LOG_LEVEL` | `info` | Log level, used when `RUST_LOG` is unset |
| `RUST_LOG` | unset | Full tracing filter; wins over `LOG_LEVEL` |
| `LOG_FORMAT` | `pretty` | `pretty` or `json` (Docker: `json`) |
| `DATABASE_URL` | unset | `sqlite:…` or `postgres://…` (secret); unset = SQLite `<DATA_DIR>/cloudit.db` |
| `DB_MAX_CONNECTIONS` | 8 (SQLite) / 10 (PostgreSQL) | Pool size per instance |
| `STORAGE` | `fs` | `fs` or `s3` |
| `STORAGE_FS_DIR` | `<DATA_DIR>/blobs` | Blob directory for `fs` (created) |
| `S3_ENDPOINT` | unset (AWS) | S3 API endpoint as seen by the server; `http://` allowed |
| `S3_REGION` | `us-east-1` | |
| `S3_BUCKET` | — | Required with `STORAGE=s3`; must exist |
| `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` | unset | Both or neither (then AWS environment credentials); the secret is a `Secret` |
| `S3_FORCE_PATH_STYLE` | `true` | Path-style URLs (MinIO, Garage); `false` = virtual-hosted |
| `S3_PRESIGN` | `false` | Hand out presigned URLs; requires `STORAGE=s3` |
| `S3_PUBLIC_ENDPOINT` | `S3_ENDPOINT` | Endpoint as reached by clients; presigned URLs are signed for it |
| `S3_PRESIGN_TTL` | `15m` | Presigned URL validity (`900`, `15m`, `1h`…; 1 s to 7 d) |
| `REDIS_URL` | unset | `redis://…` (secret), Redis or Valkey; optional, never fatal; `rediss://` unsupported |
| `TEST_DATABASE_URL`, `TEST_S3_*`, `TEST_REDIS_URL` | unset | Tests only (`make test-ha`, CI): run the server tests against PostgreSQL / S3 / Redis instead of SQLite and skipping |
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
`up`/`down`, `up-ha`/`down-ha` (HA example stack; `down-ha` keeps volumes), `test-ha`
(`scripts/test-ha.mjs`: postgres + garage + valkey from `docker-compose.ha.yml` as project
`cloudit-test` on random 127.0.0.1 ports, `cargo test -p cloudit-server` with the `TEST_*` env, then
`down -v`; `--keep` leaves them running), `clean`, `version V=x.y.z`.

Server CLI (`cloudit-server <command>`; `help` lists them): `setup-token` (prints the pending
first-run token), `admin reset-password <email>` (prints a 24-hour reset path, plus the URL with
`PUBLIC_URL`), `healthcheck`. They use the server's configuration and database but never migrate
or serve.

## Planned design (NOT implemented yet)

Decided design for the next milestones, kept here so they don't have to re-derive it. Move each part
into the sections above as it gets built.

Milestones: 1 scaffolding (done) · 2 core chunking/manifest (done) · 3a server infrastructure
(DB, storage, Redis, readiness: done) · 3b password accounts, sessions, devices, admin API (done) ·
3c identity providers (OIDC / OAuth2) ·
4 scopes/folders/sharing/sync protocol/events/jobs · 5 web UI (from mockups) · 6 desktop manual
sync + conflicts · 7 desktop auto sync/tray/autostart/installers.

Planned layout additions: `apps/desktop` (Tauri 2 + Svelte 5, milestone 6; OS code in a `platform/`
module).

### Storage and deployment

Database, blob storage, Redis, the rate limiter and the HA compose are implemented (see "Data and
state"). Still planned: Redis pub/sub for events across instances (fallback: polling); background
jobs on top of `db::lease` (GC, retention); presigned or proxied blob transfer in the sync API.

### Data model

- user > scope (user-named group) > folder (synced unit) > snapshots (immutable versions).
- Scopes are personal but shareable with other users (roles `viewer` / `editor`).
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

### Auth (milestone 3c: identity providers)

Built on the 3b tables (`users.password_hash` nullable, `admin_from_groups`, `settings`,
`AuthConfig.providers`).

- **Providers** are configured from the web UI by admins and stored in the database (not in env):
  OIDC presets google, microsoft, gitlab, apple, authentik, keycloak, authelia, zitadel, pocketid
  and generic OIDC; OAuth2 github and discord. Each has a durable provider id (referenced by the
  identities, never reused), a display name, an icon (preset or generic), the groups claim
  (default `groups`), admin groups and allowed groups. Group-derived admin
  (`admin_from_groups`) is recomputed at each login through that provider.
- **Client secrets are encrypted at rest** with `SECRET_KEY` (env; required with more than one
  instance) or, single instance only, an auto-generated `<DATA_DIR>/secret.key`.
- **Settings in the database**, editable by admins: registration mode, email auto-link (off by
  default; only for emails the provider marks verified, from trusted providers), public URL (env
  `PUBLIC_URL` overrides it).
- **Onboarding** on the first login of an unknown identity: "create a new account" (name
  prefilled; email from the provider, else asked and required; password optional) or "I already
  have an account" (sign in with an existing method, then the identity is linked).
- Email + password login is always available and cannot be disabled. Admins can create
  password-reset links for identity-provider-only accounts too (the reset sets a password).
