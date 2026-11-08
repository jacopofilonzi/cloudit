# Project structure

cloudit is a self-hosted folder sync service: a Rust HTTP server stores versioned snapshots of
users' folders; a desktop client (planned) syncs local folders with it, and a web UI (served by the
server as a static SPA) manages accounts, scopes and folders. The UIs talk to the server only
through its JSON API under `/api/v1`, whose TypeScript types are generated from the Rust ones.

Current state (milestone 4 done): the server has its infrastructure (database with migrations and
leases, blob storage, optional Redis with a rate limiter, readiness checks), exposes health probes,
serves the web SPA and has the account API: first-run setup, password accounts with web sessions,
invites, admin approval of new accounts, password resets, desktop device login (loopback +
PKCE), admin endpoints, and sign-in through identity providers (OIDC presets, GitHub,
Discord) configured by admins, with onboarding and identity linking. It also has scopes,
folders, sharing with roles, the trash and the retention settings, and the sync protocol:
chunk transfer (proxied or presigned), snapshot commits with conflicts, history, restore,
pin / label, ZIP downloads, storage usage and quotas; real-time events (SSE, Redis pub/sub
across instances, `/sync/heads` polling fallback) and the background jobs (retention,
garbage collection, orphan sweep, trash purge with permanent delete, cleanup). API only: the web pages come with the
mockups of milestone 5 (the web page shows the server status).
`cloudit-core` implements the sync formats (chunking, chunk encoding, manifest, path rules, diff;
see "Sync formats"); the server verifies every upload with it. Everything under "Planned
design" is not implemented yet.

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
│   │                 ComponentStatus), ErrorCode, ApiErrorBody {code, message, field?,
│   │                 head_snapshot_id?, hashes?, detail?}
│   ├── sensitive.rs  Sensitive: password/token string in API bodies, Debug prints `***`
│   ├── auth.rs       RegistrationMode, NewAccounts, ApprovalStatus, SignupPending, AuthConfig,
│   │                 AuthProvider, SetupStatus, SetupRequest, RegisterRequest, LoginRequest,
│   │                 PasswordResetRequest, InviteLookupRequest, InvitePreview
│   ├── me.rs         UserProfile, UpdateMeRequest, ChangePasswordRequest, SessionInfo
│   ├── devices.rs    TokenTtl (1h/1d/7d/30d/1y/forever), CodeChallengeMethod, DeviceAuthorize*,
│   │                 DeviceToken*, DeviceInfo, UpdateDeviceRequest
│   ├── admin.rs      AdminUser, AdminUserList, AdminUserUpdate, PasswordResetLink,
│   │                 CreateInviteRequest, InviteLink, InviteStatus, InviteInfo, ServerSettings,
│   │                 UpdateSettingsRequest, AdminSummary
│   ├── providers.rs  ProviderKind (+ as_str/parse), ProviderProtocol, ProviderPreset,
│   │                 ProviderInfo, CreateProviderRequest, UpdateProviderRequest, ProviderTestResult
│   ├── oauth.rs      OAuthPurpose, OAuthStartRequest, OAuthStartResponse, IdentityInfo,
│   │                 IdentityProviderRef
│   ├── onboarding.rs OnboardingInfo, OnboardingProvider, OnboardingCreateRequest,
│   │                 OnboardingLinkRequest
│   ├── scopes.rs     ScopeRole (ordered), MemberRole (default editor), ScopeUser, ScopeInfo,
│   │                 Create/UpdateScopeRequest, RestoreRequest, ScopeMember, AddMemberRequest,
│   │                 UpdateMemberRequest, TrashItemKind, TrashItem, TrashEmptyResponse
│   ├── folders.rs    FolderInfo (+ head: FolderHead, unresolved_conflicts), CreateFolderRequest,
│   │                 UpdateFolderRequest, Patch<T> (omitted / null / value)
│   ├── sync.rs       ChunkSpec, ChunkCheckRequest/Response, MissingChunk, Transfer (+ TransferKind
│   │                 proxy | presigned), ChunkDownloadRequest/Response, ChunkDownload,
│   │                 MAX_CHUNKS_PER_REQUEST (10 000), MAX_STORED_CHUNK_BYTES (1 MiB + 1 + 64 KiB)
│   ├── snapshots.rs  SnapshotKind, SnapshotInfo, CommitResponse, SnapshotList,
│   │                 UpdateSnapshotRequest, RestoreSnapshotRequest
│   ├── usage.rs      UsageInfo, ScopeUsage
│   ├── events.rs     ServerEvent (tag `type` + `data`; name(), data_json()), HelloEvent,
│   │                 RealtimeMode, FolderHeadEvent, ConflictEvent, HistoryChangedEvent,
│   │                 ChangeKind, FolderChangedEvent, ScopeChangedEvent,
│   │                 MembershipChangedEvent, UsageChangedEvent, SyncHead, SyncHeads
│   └── jobs.rs       JobName (ALL, as_str/parse), JobOutcome, JobRunInfo, JobInfo
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
│                     job scheduler (if JOBS_ENABLED) → router → serve (with peer addresses)
│                     with graceful shutdown
├── cli.rs            subcommands: setup-token, admin reset-password <email>, gen-secret-key,
│                     healthcheck, help
├── lib.rs            module list (library so tests can build the router)
├── state.rs          AppState: config, db, BlobStore, Option<Redis>, RateLimiter, instance_id,
│                     Leases, setup_done, secrets (SecretBox), idp, events (Events),
│                     job_wakeup (Notify); `init` resolves the secret
│                     key, connects + migrates, checks the key canary, setup::init (fatal on
│                     error); Redis is never fatal; `public_url()` = effective public URL
├── validate.rs       email (trim + lowercase + shape), text / display_name (trimmed, bounded),
│                     public_url (http(s)://host[:port] only), range, retention (0..=100000), emoji
│                     (≤ 32 bytes, no letters / whitespace / controls)
├── time.rs           now (µs precision), add, rfc3339 (API timestamps `2026-01-31T12:00:00Z`)
├── platform.rs       write_new_private_file (0600 on Unix): with shutdown.rs the only OS-specific code
├── hooks.rs          test pause points (Point; `arm` under cfg(test), no-op otherwise) for forced races
├── events/           real-time events (design in mod.rs docs)
│   ├── mod.rs        Events: publish (local broadcast + Redis outbox), subscribe, realtime(),
│   │                 open_stream (StreamSlot, 20 per user and instance), Delivery
│   ├── fanout.rs     Redis publisher task (PUBLISH cloudit:events) and subscriber task (one
│   │                 dedicated pub/sub connection, ping 15 s, reconnect 1–30 s backoff)
│   └── notify.rs     scope_recipients (owner + members) and one helper per event
├── jobs/             background jobs (rules in mod.rs docs)
│   ├── mod.rs        interval(job), Counts, RunContext (lease lost → check() fails), run_job,
│   │                 request (jobs row + local wake-up)
│   ├── scheduler.rs  start / start_with (tick 15 s + jitter), is_due, run_leased (lease
│   │                 job:<name>, TTL 5 min renewed every minute, run recorded)
│   ├── store.rs      jobs / job_runs: ensure_rows, all, get, request, start (closes stale
│   │                 runs as interrupted), finish, recent_runs, delete_runs_before, run_info
│   ├── retention.rs  select (pure, table-tested), delete_snapshot (locks the row), apply, run
│   ├── gc.rs         unreferenced rows → tombstones + usage in one transaction, then
│   │                 delete_tombstoned (objects under the scope lock), delete_objects
│   ├── orphans.rs    orphan_sweep: list scopes/*, old objects without row, delete under the lock
│   ├── trash_purge.rs purge_scope (rows, then objects by prefix), purge_folder, run
│   └── cleanup.rs    expired sessions, codes, flows, pending sign-ups, resets, old unused
│                     invites, old job runs
├── crypto/           server key and encryption of stored secrets
│   ├── mod.rs        SecretBox: XChaCha20-Poly1305, `[1][nonce 24][ciphertext+tag]`, AAD per value
│   ├── key.rs        SecretKey (32 bytes, base64); resolve: SECRET_KEY, else <DATA_DIR>/secret.key
│   │                 (SQLite only), else refuse (PostgreSQL)
│   └── canary.rs     settings `secret_key_check`: insert-if-absent, decrypt at startup
├── accounts/         account data (queries the routes call); single-use token rules in mod.rs
│   ├── users.rs      insert (email_taken on unique violation), find, find_by_email,
│   │                 set_password, update_profile, grant_admin_from_groups, list (approval
│   │                 filter) + count, count_pending, approval_for_signup, ensure_active
│   ├── admin.rs      update_flags (admin / disabled; disabling deletes the web sessions),
│   │                 set_admin_from_groups, approve / reject (one conditional UPDATE each;
│   │                 reject deletes the web sessions), with the race-free last-admin guard
│   ├── setup.rs      setup_state: init at startup (same token on every instance), status,
│   │                 check_token, complete (conditional UPDATE)
│   ├── settings.rs   settings table: registration_mode (default invite), password_new_accounts
│   │                 (default approval), autolink_email (default false), public_url,
│   │                 retention_defaults (20 / 5), trash_retention_days (30),
│   │                 default_quota_bytes (unlimited);
│   │                 get_strict, insert_if_absent
│   ├── invites.rs    create, find_usable(_by_hash), consume (conditional UPDATE), list, revoke, status
│   └── password_resets.rs  create (24 h; marks the user's older unused resets used), consume
├── auth/             authentication (the module docs list the pieces)
│   ├── tokens.rs     new_token (32 random bytes, base64url), cld_ device tokens, hash_token
│   │                 (SHA-256 hex), pkce_challenge, constant_time_eq
│   ├── password.rs   argon2id (m=19 MiB, t=2, p=1) on the blocking pool; verify_or_dummy
│   ├── sessions.rs   cloudit_session cookie; create, authenticate, slide_if_due, list, delete
│   ├── client.rs     ClientInfo: client IP, https, own origin, user agent
│   ├── csrf.rs       Origin / Sec-Fetch-Site check
│   ├── middleware.rs /api/v1 layer: setup gate, CSRF, Auth (bearer or cookie), cookie refresh
│   ├── extractors.rs SessionUser, AnyUser, AdminUser, Client
│   └── limits.rs     rate limits of the auth endpoints and of sharing + enforce
├── scopes/           scopes, members, folders, trash (database side; rules in mod.rs docs)
│   ├── access.rs     ScopeAccess / FolderAccess / SnapshotAccess: load (404 if invisible),
│   │                 load_trashed, require(role) (403); the only access check
│   ├── names.rs      name rules (validate::text, 100 chars) + lowercase key
│   ├── store.rs      scopes: create, find, list_visible, folder_counts, rename, trash,
│   │                 restore, trashed_owned, managed
│   ├── members.rs    scope_members: role_of, find, list, upsert, update_role, remove
│   └── folders.rs    folders: create, find, list, update (name / retention), trash, restore,
│                     trashed_in
├── sync/             sync protocol (object layout, invariants and the GC contract in mod.rs docs)
│   ├── keys.rs       scopes/{scope}/chunks/{hash}, scopes/{scope}/manifests/{hash}
│   ├── objects.rs    the writer/collector protocol (scope lock, race argument in the docs):
│   │                 Kind, parse_key, lock_scope, row_exists, Stored, ensure_present
│   ├── usage.rs      effective_quota, owner_quota, used_by(_many), ensure_room (estimate),
│   │                 insert_charged (scope lock, object check, row + scopes.stored_bytes in
│   │                 one transaction, exact quota under an owner lock → Charged), recompute
│   ├── chunks.rs     sizes / exists (batched IN lists), decode (blocking pool), adopt (verify an
│   │                 object without row: HEAD, bounded GET, decode, then insert_charged with
│   │                 the HEAD metadata; invalid → deleted under the scope lock)
│   ├── manifests.rs  decode (bounded, blocking pool), exists, store, stored_bytes, load
│   ├── snapshots.rs  find, commit (insert + snapshot_chunks + conditional head), resolve, list,
│   │                 update (pin / label), restore (locks the source row, INSERT … SELECT
│   │                 copy), folder_sync
│   ├── transfer.rs   Transfers: presigned URL or proxy URL (absolute with a public URL)
│   └── archive.rs    streaming ZIP (async_zip over a duplex pipe), filename, Content-Disposition
├── idp/              identity providers (protocol side)
│   ├── mod.rs        Idp: shared HttpClient, Discovery cache, GitHub/Discord endpoints
│   │                 (replaceable only under cfg(test)); discover → provider_discovery_failed
│   ├── presets.rs    PRESETS (one per ProviderKind), icons, Microsoft issuer, refused tenants
│   ├── fields.rs     ProviderFields (admin-editable settings) and their validation rules
│   ├── store.rs      auth_providers queries; Provider (decoded row, client_secret decrypt,
│   │                 issuer, admin and login-page views); soft delete
│   ├── http.rs       HttpClient: reqwest, 5 s connect / 15 s timeout, no redirects, 1 MiB
│   │                 bodies; openidconnect adapter; get_json, post_form_auth
│   ├── oidc.rs       Discovery: metadata + JWKS per issuer, 1 h in-memory cache, refresh_jwks
│   ├── signin.rs     authorize_url, sign_in: code exchange, ID token verification, user info
│   ├── social.rs     GitHub (/user, /user/emails) and Discord (/users/@me) identities
│   └── claims.rs     ExternalIdentity (normalized email and name, groups), group rules
├── oauth/            provider sign-in state (database) and outcomes
│   ├── mod.rs        return_to validation
│   ├── flows.rs      oauth_flows: create, consume (one conditional UPDATE); Purpose
│   ├── cookies.rs    cloudit_oauth (browser binding) and cloudit_onboarding cookies
│   ├── identities.rs identities: find, insert (identity_in_use), record_sign_in, list,
│   │                 provider_ids_by_user, unlink (last_login_method guard)
│   ├── pending.rs    pending_signups: create, find_usable(_by_id), consume, identity
│   └── complete.rs   callback outcomes by purpose (login, link, onboarding_link), apply_groups
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
│   │                 000003 tokens (invites, password_resets, devices, device_auth_codes),
│   │                 000004 auth_providers, 000005 identities (identities, pending_signups,
│   │                 oauth_flows), 000006 account approval (users.approval_*,
│   │                 signup_source, auth_providers.new_accounts), 000007 scopes
│   │                 (scopes, scope_members, folders), 000008 sync (chunks, manifests,
│   │                 snapshots, snapshot_chunks, scopes.stored_bytes, users.quota_bytes),
│   │                 000009 jobs and gc (jobs, job_runs, object_deletions,
│   │                 scopes/folders.purge_requested_at), 000010 folder emoji
│   ├── entities/     SeaORM 2 models (`#[sea_orm::model]`, no relations), one file per table
│   │                 (none for setup_state: its plaintext token is read as a Secret);
│   │                 oauth_flows keeps verifier and nonce in `Hidden` (Debug prints `***`)
│   ├── lease.rs      Leases: try_acquire / renew / release, one atomic statement each
│   └── testing.rs    tests only: TestDb (SQLite temp file, or a fresh database on TEST_DATABASE_URL)
├── storage/
│   ├── mod.rs        BlobStore over object_store: put, get (stream), get_bytes, head, delete, list,
│   │                 presign_put / presign_get, check (readiness)
│   └── tests.rs      fs round trip, offline presign checks, live S3 test (TEST_S3_*)
├── cache/
│   ├── mod.rs        Redis: lazy ConnectionManager, `run` with 2 s timeout + 5 s backoff, rate-limited
│   │                 warnings, client() for pub/sub, is_available()
│   └── rate_limit.rs RateLimiter: fixed window in Redis (shared) or in memory (fallback)
├── errors/mod.rs     ApiError → JSON `{code, message, field?}` with HTTP status (+ Retry-After);
│                     internal causes logged, not returned
├── routes/
│   ├── mod.rs        router: probes, `/api/v1` (behind auth::middleware), JSON 404 under
│   │                 `/api`, SPA fallback from WEB_DIR
│   ├── extract.rs    ApiJson (rejections as bad_request / payload_too_large), body_bytes (raw
│   │                 body with a route limit), optional_json (empty body = default), parse_id,
│   │                 model → API conversions
│   ├── health.rs     /health (liveness), /ready (database + storage checks, Redis reported)
│   ├── setup.rs      /setup
│   ├── auth.rs       /auth/* (config, register, login, logout, password-reset)
│   ├── oauth.rs      /auth/oauth/{provider}/start and /callback; begin (shared flow start)
│   ├── onboarding.rs /auth/onboarding* (info, create, link with password or provider)
│   ├── me.rs         /me, /me/password, /me/sessions, /me/identities
│   ├── usage.rs      /me/usage
│   ├── devices.rs    /devices (authorize, token, list, rename, revoke)
│   ├── admin.rs      /admin/* (users, approve / reject, password resets, invites, settings,
│   │                 summary)
│   ├── providers.rs  /admin/providers* (presets, CRUD, test)
│   ├── scopes.rs     /scopes* (CRUD, trash, restore) and /scopes/{id}/members*
│   ├── folders.rs    /scopes/{id}/folders and /folders/{id}* (CRUD, trash, restore)
│   ├── trash.rs      /trash, /trash/scopes/{id}, /trash/folders/{id}, /trash/empty
│   ├── events.rs     /events (SSE stream, credential re-check at each keep-alive), /sync/heads
│   ├── jobs.rs       /admin/jobs, /admin/jobs/{name}/run
│   ├── chunks.rs     /folders/{id}/chunks/check, /download, /{hash} (proxy PUT / GET)
│   ├── snapshots.rs  /folders/{id}/snapshots (commit, history), /snapshots/{id} (get, patch),
│   │                 /manifest, /restore, /resolve, /archive
│   └── tests/        router tests (tower `oneshot`, real AppState; PostgreSQL with
│                     TEST_DATABASE_URL): mod.rs (probes, SPA), support.rs (TestApp harness),
│                     fake_idp.rs (fake OIDC / GitHub / Discord server; test-only RSA keys in
│                     fixtures/), setup, auth, me, devices, admin, providers, oauth,
│                     onboarding, approval, secret_key, scopes (Team fixture, role matrix,
│                     sharing, names, races), folders (trash, retention), sync_client.rs
│                     (test sync client on cloudit-core), chunks, snapshots (commits,
│                     conflicts, validation), history (round trip, pages, pin, restore),
│                     archive, quota, presign (live S3 with TEST_S3_*), events (SSE
│                     reader, recipients, revocation, limits, Redis fan-out), jobs
│                     (scheduler, leases, cleanup), retention, gc (forced races with
│                     hooks), trash_purge
├── logging.rs        tracing subscriber (env filter; pretty or json)
├── shutdown.rs       Ctrl+C / SIGTERM future (OS-specific, like platform.rs)
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
Bearer cld_…`); **admin** (session of an effective admin); **onboarding** (the
`cloudit_onboarding` cookie of a live pending signup, else 410 `onboarding_expired`).

| Method and path | Auth | Body → answer |
| --- | --- | --- |
| `GET /health` | none | `HealthResponse` |
| `GET /setup` | none | `SetupStatus {required}` |
| `POST /setup` | none, rate limited | `SetupRequest {…, public_url?}` → 201 `UserProfile` + cookie (first admin, email verified) |
| `GET /auth/config` | none | `AuthConfig {setup_required, registration_mode, password_requires_approval, providers: AuthProvider[] {id, kind, display_name, icon}}` (enabled, non-deleted, position order) |
| `POST /auth/register` | none, rate limited | `RegisterRequest` → 201 `UserProfile` + cookie, or 202 `SignupPending {status: "pending"}` (no session) when the account needs approval |
| `POST /auth/login` | none, rate limited | `LoginRequest` → 200 `UserProfile` + cookie |
| `POST /auth/logout` | none | → 204; deletes the current session, clears the cookie |
| `POST /auth/password-reset` | none, rate limited | `PasswordResetRequest` → 204 (no sign-in) |
| `POST /auth/invites/lookup` | none, rate limited | `InviteLookupRequest {token}` → `InvitePreview {email?, expires_at, invited_by}` (the inviter's display name); unknown, used, revoked or expired → 400 `invite_invalid`. POST so the token stays out of request logs |
| `POST /auth/oauth/{provider}/start` | none (`link`: session), rate limited, CSRF | `OAuthStartRequest {purpose: login\|link, return_to?, invite_token?}` → `{redirect_url}` + `cloudit_oauth` cookie |
| `GET /auth/oauth/{provider}/callback?code&state` | `cloudit_oauth` cookie | 302 to `return_to` (+ session cookie), `/onboarding` (+ `cloudit_onboarding`), or `/login?error=<code>` |
| `GET /auth/onboarding` | onboarding | `OnboardingInfo` (`invite_required`, `requires_approval`) |
| `POST /auth/onboarding/create` | onboarding, rate limited (as register) | `OnboardingCreateRequest {display_name, email?, password?, invite_token?}` → 201 `UserProfile` + cookie, or 202 `SignupPending` (no session, onboarding cookie cleared) when the account needs approval |
| `POST /auth/onboarding/link` | onboarding, rate limited (as login) | `OnboardingLinkRequest {email, password}` → 200 `UserProfile` + cookie |
| `POST /auth/onboarding/link/{provider}/start` | onboarding, rate limited | no body → `{redirect_url}` + `cloudit_oauth` cookie (`onboarding_link` flow) |
| `GET /me` | any | `UserProfile` |
| `PATCH /me` | session | `UpdateMeRequest` → `UserProfile` |
| `POST /me/password` | session | `ChangePasswordRequest` → 204; other sessions signed out |
| `GET /me/sessions` | session | `SessionInfo[]` (current one marked) |
| `DELETE /me/sessions/{id}` | session | → 204 |
| `GET /me/identities` | session | `IdentityInfo[]` (oldest first; disabled / deleted providers flagged) |
| `DELETE /me/identities/{id}` | session | → 204; 409 `last_login_method` if no password and no other identity on an enabled provider would remain |
| `POST /devices/authorize` | session | `DeviceAuthorizeRequest` → `{redirect_url}` |
| `POST /devices/token` | none, rate limited, no CSRF check | `DeviceTokenRequest` → `DeviceTokenResponse` |
| `GET /devices` | session | `DeviceInfo[]` (revoked and expired included) |
| `PATCH /devices/{id}` | session | `UpdateDeviceRequest` → `DeviceInfo` |
| `DELETE /devices/{id}` | session | → 204 (revoke, soft) |
| `GET /admin/users?limit=&offset=&approval_status=` | admin | `AdminUserList` (oldest first; limit 1..=200, default 50; optional filter `approved` / `pending` / `rejected`, `total` counts the matches; each with `identity_providers`, `has_password`, `approval_status`, `signup_source`, `approval_decided_at`, `approval_decided_by`, `used_bytes`, `quota_bytes` (own), `effective_quota_bytes`) |
| `PATCH /admin/users/{id}` | admin | `AdminUserUpdate {admin?, disabled?, quota_bytes? (null = server default)}` → `AdminUser`; disabling deletes the web sessions |
| `POST /admin/users/{id}/approve` | admin | → `AdminUser`; pending or rejected (lifts the ban) → approved; already approved: unchanged |
| `POST /admin/users/{id}/reject` | admin | → `AdminUser`; pending or approved → rejected, web sessions deleted; own account → 403 `forbidden`; 409 `last_admin`; already rejected: unchanged |
| `GET /admin/summary` | admin | `AdminSummary {pending_users}` |
| `POST /admin/users/{id}/password-reset` | admin | → 201 `PasswordResetLink {token, path: "/reset-password/<token>", expires_at}`; older unused resets of the user stop working |
| `GET /admin/invites` | admin | `InviteInfo[]` (newest first, no tokens) |
| `POST /admin/invites` | admin | `CreateInviteRequest {email?, expires_in?}` (default 7d; `forever` → 422) → 201 `InviteLink {id, token, path: "/invite/<token>", expires_at}` |
| `DELETE /admin/invites/{id}` | admin | → 204 (revoke, soft) |
| `GET /admin/settings` | admin | `ServerSettings {registration_mode, password_new_accounts, autolink_email, public_url, public_url_from_env, effective_public_url, retention_max_versions, retention_max_per_day, trash_retention_days, default_quota_bytes}` |
| `PATCH /admin/settings` | admin | `UpdateSettingsRequest {registration_mode?, password_new_accounts?, autolink_email?, public_url? ("" clears), retention_max_versions? (0..=100000, 0 = unlimited), retention_max_per_day? (same), trash_retention_days? (1..=3650), default_quota_bytes? (null = unlimited)}` → `ServerSettings`; all fields validated before any is written |
| `GET /admin/providers/presets` | admin | `ProviderPreset[]` (one per kind) |
| `GET /admin/providers` | admin | `ProviderInfo[]` (non-deleted, position order; `has_secret`, never the secret; `callback_url` when a public URL is known; `new_accounts` `auto` / `approval`) |
| `POST /admin/providers` | admin | `CreateProviderRequest` (`new_accounts` default `approval`) → 201 `ProviderInfo`; OIDC: discovery must succeed (502 `provider_discovery_failed`) |
| `PATCH /admin/providers/{id}` | admin | `UpdateProviderRequest` (no id/kind; secret optional) → `ProviderInfo`; a changed issuer is discovered again |
| `DELETE /admin/providers/{id}` | admin | → 204 (soft: `deleted_at`, disabled; identities kept, id never reused) |
| `POST /admin/providers/{id}/test` | admin | → `ProviderTestResult {ok, code?}` (fresh discovery + secret decryption, read-only) |
| `GET /scopes` | any | `ScopeInfo[] {id, name, role, owner: ScopeUser {id, email, display_name}, folder_count, created_at, updated_at}`: owned + shared, outside the trash, by name (case-insensitive) |
| `POST /scopes` | any | `CreateScopeRequest {name}` → 201 `ScopeInfo` (caller = owner); 409 `name_taken` |
| `GET /scopes/{id}` | any; viewer | `ScopeInfo` |
| `PATCH /scopes/{id}` | session; owner | `UpdateScopeRequest {name}` → `ScopeInfo`; 409 `name_taken` |
| `DELETE /scopes/{id}` | session; owner | → 204: to the trash (its folders become unreachable) |
| `POST /scopes/{id}/restore` | session; owner of the trashed scope | optional `RestoreRequest {name?}` → `ScopeInfo`; 409 `name_taken` if the name was taken meanwhile (retry with `name`) |
| `GET /scopes/{id}/members` | any; viewer | `ScopeMember[] {user, role, added_by, created_at}`: the owner first (role `owner`), then by date added |
| `POST /scopes/{id}/members` | session; owner; 30 / 15 min per user | `AddMemberRequest {email, role? (default editor)}` → 201 `ScopeMember`, or 200 when already a member (role changed); 404 `user_not_found` unless an approved, enabled account has that email; the owner's own email → 422 |
| `PATCH /scopes/{id}/members/{user_id}` | session; owner | `UpdateMemberRequest {role}` → `ScopeMember`; not a member (incl. the owner) → 404 |
| `DELETE /scopes/{id}/members/{user_id}` | session; owner, or the member itself (leave) | → 204; the owner can't leave (403) |
| `GET /scopes/{id}/folders` | any; viewer | `FolderInfo[]` outside the trash, by name |
| `POST /scopes/{id}/folders` | any; editor (retention overrides: admin) | `CreateFolderRequest {name, emoji?, retention_max_versions?, retention_max_per_day?}` → 201 `FolderInfo`; 409 `name_taken` |
| `GET /folders/{id}` | any; viewer | `FolderInfo {id, scope_id, name, emoji, head_snapshot_id, head? {snapshot_id, created_at, logical_size, file_count}, unresolved_conflicts, retention_max_versions, retention_max_per_day, effective_retention_max_versions, effective_retention_max_per_day, created_by, created_at, updated_at}` |
| `PATCH /folders/{id}` | session; editor (retention fields: admin) | `UpdateFolderRequest {name?, emoji?, retention_max_versions?, retention_max_per_day?}` (emoji: omitted = unchanged, `null` / `""` = none; retention: omitted = unchanged, `null` = server default) → `FolderInfo` |
| `DELETE /folders/{id}` | session; admin | → 204: to the trash |
| `POST /folders/{id}/restore` | session; admin of its scope (outside the trash) | optional `RestoreRequest {name?}` → `FolderInfo`; 409 `name_taken` |
| `POST /folders/{id}/chunks/check` | any; editor | `ChunkCheckRequest {chunks: [{hash, size}]}` (≤ 10 000, size 1..=1 MiB, one size per hash) → `ChunkCheckResponse {missing: [{hash, upload: Transfer}]}` (by hash); 507 `quota_exceeded` if usage + the missing chunks' uncompressed sizes > quota |
| `PUT /folders/{id}/chunks/{hash}` | any; editor | body = stored chunk (≤ `MAX_STORED_CHUNK_BYTES`, else 413) → 204; 422 `chunk_invalid` unless it decodes to `hash`; 507 `quota_exceeded`; idempotent |
| `POST /folders/{id}/chunks/download` | any; viewer | `ChunkDownloadRequest {hashes}` (≤ 10 000) → `ChunkDownloadResponse {chunks: [{hash, download: Transfer}], missing}` |
| `GET /folders/{id}/chunks/{hash}` | any; viewer | the stored chunk bytes (`application/octet-stream`); 404 unless the scope has it |
| `POST /folders/{id}/snapshots?parent=<id\|none>&kind=normal\|conflict` | any; editor | body = stored manifest (≤ 64 MiB) → 201 `CommitResponse {unchanged: false, snapshot}`, or 200 `{unchanged: true, snapshot: head}`; 409 `conflict` (+ `head_snapshot_id`), 422 `manifest_invalid` (+ `detail`), `chunk_missing` / `chunk_invalid` (+ `hashes`), `validation_failed` (`parent` not a snapshot id or a snapshot of another folder, `kind` restore), 507 `quota_exceeded`; an unknown `parent` id (e.g. deleted by retention) → 409 `conflict` for `normal`, kept as a dangling `parent_id` for `conflict` |
| `GET /folders/{id}/snapshots?cursor=&limit=&kind=&unresolved=` | any; viewer | `SnapshotList {snapshots, next_cursor}` (newest first; limit 1..=200, default 50) |
| `GET /snapshots/{id}` | any; viewer | `SnapshotInfo` |
| `PATCH /snapshots/{id}` | any; editor | `UpdateSnapshotRequest {pinned?, label?}` (label ≤ 100 characters; `null` / `""` clears) → `SnapshotInfo` |
| `GET /snapshots/{id}/manifest[?format=json]` | any; viewer | stored manifest bytes, or the canonical JSON (`Manifest`); `ETag` = `"<manifest hash>"`, `If-None-Match` → 304 |
| `POST /snapshots/{id}/restore` | any; editor | optional `RestoreSnapshotRequest {expected_head?}` → 201 `SnapshotInfo` (kind `restore`, the new head); 409 `conflict` if `expected_head` isn't the head; a conflict source gets resolved |
| `POST /snapshots/{id}/resolve` | any; editor | → `SnapshotInfo` (conflicts only, else 422; idempotent): "keep server" |
| `GET /snapshots/{id}/archive` | any; viewer | streaming ZIP (`application/zip`, `<folder>-<YYYYMMDD-HHMMSS>.zip`) |
| `GET /me/usage` | any | `UsageInfo {used_bytes, quota_bytes (effective, null = unlimited), scopes: [{id, name, stored_bytes, trashed}]}` (owned scopes, by name) |
| `GET /trash` | session | `TrashItem[] {kind: scope\|folder, id, name, scope_id, scope_name, deleted_at, deleted_by, purge_at}`: own trashed scopes + trashed folders of scopes where the caller is owner or admin, not marked for permanent deletion; newest first |
| `DELETE /trash/scopes/{id}` | session; owner of the trashed scope | → 202: marks it for permanent deletion (`purge_requested_at`; no longer listed or restorable) and requests a `trash_purge` run; not in the trash or already marked → 404 |
| `DELETE /trash/folders/{id}` | session; owner / admin of its scope (outside the trash) | → 202: the same for a trashed folder |
| `POST /trash/empty` | session | → 202 `TrashEmptyResponse {scopes, folders}`: marks everything `GET /trash` lists |
| `GET /events` | any | `text/event-stream` (see "Real-time events"); 20 open streams per user and instance, more → 429 `rate_limited` |
| `GET /sync/heads` | any | `SyncHeads {folders: [{scope_id, folder_id, head_snapshot_id, head_created_at, unresolved_conflicts}]}`: every visible folder outside the trash, by folder id; `ETag` (hash of the body), `If-None-Match` → 304 |
| `GET /admin/jobs` | admin | `JobInfo[] {name, interval_secs, requested_at, last_started_at, recent_runs: JobRunInfo[] {id, job, instance_id, started_at, finished_at, outcome, counts, error}}` (≤ 10 runs, newest first; outcome `running` / `succeeded` / `failed` / `interrupted`), in the order retention, gc, orphan_sweep, trash_purge, cleanup |
| `POST /admin/jobs/{name}/run` | admin | → 202 `JobInfo`: sets `requested_at`; a scheduler runs it shortly (this instance at once if its jobs are enabled); unknown name → 404 |

Scope routes: "viewer", "editor", "admin", "owner" is the minimum scope role; a scope or
folder the caller can't see answers 404 `not_found` (also with a device token), a visible one
with a role too low 403 `forbidden`; session-only routes refuse device tokens with 403 first.

Any other path under `/api` → 404 `{"code": "not_found", "message": …}`. Every other path is served
from `WEB_DIR` with `index.html` fallback (or a JSON 404 if `WEB_DIR` is missing).

Errors: JSON `ApiErrorBody {code, message, field?}`; `message` is English, for logs only; `field`
names the request field with `validation_failed`. Codes (`ErrorCode`) and statuses: `bad_request`
400 (malformed JSON, unknown fields, bad query), `not_found` 404, `unavailable` 503, `internal` 500
(generic message), `unauthorized` 401, `forbidden` 403, `invalid_credentials` 401 (wrong email or
password: same answer), `account_disabled` 403, `invite_invalid` 400,
`token_invalid` 400 (reset token, device code, PKCE verifier, redirect mismatch), `email_taken`
409, `validation_failed` 422, `rate_limited` 429 (`Retry-After` in seconds), `csrf_failed` 403,
`setup_required` 409, `setup_already_done` 409, `setup_token_invalid` 403, `last_admin` 409,
`provider_not_found` 404, `provider_id_taken` 409, `provider_discovery_failed` 502,
`provider_error` 502 (token exchange, ID token, user info: details only in logs),
`public_url_required` 409, `provider_disabled` 403, `oauth_state_invalid` 400 (unknown, used or
expired flow, other browser, session changed during `link`), `oauth_cancelled` 400 (provider
answered `access_denied`), `group_not_allowed` 403, `identity_in_use` 409, `identity_not_linked`
409, `onboarding_expired` 410, `last_login_method` 409, `account_pending` 403 (waiting for
approval), `account_rejected` 403 (rejected by an admin), `name_taken` 409 (scope or folder
name in use, ignoring case), `user_not_found` 404 (sharing with an email that has no
approved, enabled account), `conflict` 409 (the folder head moved; `head_snapshot_id`),
`chunk_missing` 422 (`hashes`, at most 1000), `chunk_invalid` 422 (`hashes`),
`manifest_invalid` 422 (`detail` = the core code, e.g. `manifest_not_sorted`, or
`manifest_chunk_size_wrong` when a size disagrees with the stored chunk), `quota_exceeded`
507 (Insufficient Storage, as WebDAV / Nextcloud: distinct from 413 for one oversized
request and 409 for head conflicts), `payload_too_large` 413. The callback carries the code in
`/login?error=<code>` instead of a JSON body.

### Authentication

- **First-run setup**: at startup, if there is no user, every instance runs `INSERT INTO
  setup_state (1, <random token>, now) ON CONFLICT (id) DO NOTHING`, reads the row back and logs
  the token at WARN while setup is pending, so all instances print the same one (`SETUP_TOKEN`
  replaces it while pending). `POST /setup` consumes it in one transaction: `UPDATE setup_state
  SET token = NULL, completed_at = now WHERE id = 1 AND completed_at IS NULL AND token = ?`
  (exactly one winner), then creates the admin and stores `registration_mode` and the optional
  `public_url`. The row is never deleted. While pending, every `/api/v1` route except `/health`,
  `/auth/config` and `/setup` answers `setup_required` (409: a server state, not an instance
  failure); once an instance has seen setup completed it caches that. `cloudit-server
  setup-token` prints the pending token.
- **Passwords**: argon2id m=19 MiB t=2 p=1 (PHC strings), 8..=1024 bytes. Login always runs one
  verification (against a dummy hash for unknown emails or password-less accounts).
  `password_hash` is null for accounts created through a provider without a password; email +
  password login is always available and can't be disabled; an admin reset link sets a password
  for any account.
- **Sessions**: cookie `cloudit_session` = 32 random bytes base64url; `HttpOnly; SameSite=Lax;
  Path=/; Max-Age` 30 days; `Secure` when `PUBLIC_URL` is https or a trusted proxy sends
  `X-Forwarded-Proto: https`. Sliding: a session expires 30 days after its last recorded use;
  `last_seen_at` / `expires_at` are written at most once per hour (the cookie is refreshed then).
  Logout deletes the row. A password change signs out the other sessions, a password reset all of
  them (devices stay).
- **Devices** (RFC 8252 loopback + PKCE S256): `authorize` (session) checks the challenge (43
  base64url characters), the redirect URI (`http://127.0.0.1:<port>/…` or `http://[::1]:<port>/…`,
  explicit port, no query, fragment or `localhost`), `state` (1..=512 characters), name, os and
  `token_ttl` (1h, 1d, 7d, 30d default, 1y, forever), and stores a 2-minute code. `token` consumes the code
  first (conditional UPDATE), then checks the redirect URI (exact match) and SHA-256(verifier) in
  constant time: any failure burns the code. The token `cld_<base64url>` has a fixed expiry, or none
  for `forever` (`expires_at` NULL in the table and `null` in `DeviceInfo` / `DeviceTokenResponse`;
  valid until revoked, and still refused while the account is disabled or rejected);
  revoking sets `revoked_at`; `last_seen_at` is written at most every 5 minutes.
- **Disabled accounts**: disabling deletes the account's web sessions in the same transaction
  (re-enabling doesn't revive them); login (password or provider) is refused with
  `account_disabled` (after the password check); device tokens answer `account_disabled` while
  disabled and work again after re-enabling. Effective admin = `admin || admin_from_groups`.
  `PATCH /admin/users` refuses to leave no enabled effective admin (`last_admin`); concurrent
  changes are serialized by a write on the `setup_state` row as the first statement of the
  transaction (`accounts/admin.rs`). Only enabled **and approved** admins count.
- **Account approval** (`users.approval_status`): a new account is `approved` or `pending`
  when created. Approved: the first admin (setup), any sign-up that consumed a valid invite
  (password or onboarding; `signup_source = invite`), an onboarding OIDC identity in the
  provider's `admin_groups` (the provider vouches for it), and sign-ups whose policy is
  `auto`. Otherwise pending: password registrations follow the `password_new_accounts`
  setting (`signup_source = password`), onboarding follows the provider's `new_accounts`
  (`signup_source` = provider id); both default to `approval`. A pending sign-up answers 202
  `SignupPending` and creates no session. Linking an identity to an existing account
  (from `/me`, onboarding `link`, `onboarding_link`, email auto-link) never needs approval but
  requires that account to be active. Every authentication point refuses an account that is
  not enabled and approved (`users::ensure_active`, in this order: `account_disabled`,
  `account_rejected`, `account_pending`; 403): password login and onboarding `link` (only
  after the password verified), provider sign-in (`/login?error=<code>`), session and
  device-token authentication (extractors) and the device-token exchange. Admins move it with
  one conditional UPDATE (`… WHERE id = ? AND approval_status IN (…)`, recording
  `approval_decided_at/by`): `approve` from pending or rejected (lifting a ban), `reject` from
  pending or approved. Rejecting is a ban: kept forever, web sessions deleted in the same
  transaction, device tokens refused (they work again after approval), and its identities stay
  attached, so signing in with one answers `account_rejected` instead of reaching onboarding.
  `reject` takes the last-admin lock, refuses the caller's own account (`forbidden`) and the
  last enabled approved admin (`last_admin`); deciding again for the current state is a no-op.
- **Password resets**: 24 hours, single use; creating one (admin API or CLI) marks the user's
  earlier unused resets used in the same transaction.
- **CSRF**: every unsafe request (not GET/HEAD/OPTIONS) without a bearer token, except
  `POST /devices/token`, needs an `Origin` equal to the server's origin (`PUBLIC_URL`'s if set,
  else `http(s)://` + `Host`, or `X-Forwarded-Host` when trusted) or, without `Origin`,
  `Sec-Fetch-Site: same-origin`; else `csrf_failed`. Login, register, setup, provider start and
  onboarding included (login CSRF). No CORS.
- **Client IP** (rate limits): the TCP peer; with `TRUST_PROXY_HEADERS=true` the rightmost
  `X-Forwarded-For` entry that is not loopback, private or link-local (the rightmost entry if all
  are internal).
- **Rate limits** (fixed windows, so up to twice the limit may pass across a boundary; shared
  through Redis when available): login (and onboarding link) 20 / 5 min per IP and 10 / 15 min
  per email; register (and onboarding create) 10 / h per IP; setup 10 / 15 min per IP; device
  token 30 / 15 min per IP; password reset 10 / 15 min per IP; invite lookup 30 / 15 min per IP;
  current-password checks (password or email change) 10 / 15 min per user; provider
  sign-in starts 30 / 5 min per IP; adding scope members (email lookups) 30 / 15 min per user.
- **Secrets**: tokens are stored only as SHA-256 hex (`*_hash` columns) and never logged; API
  bodies carry them as `Sensitive`. The setup token is the one secret stored in plaintext (the
  CLI prints it) and logged on purpose while setup is pending. Provider client secrets are
  stored only encrypted (below); a flow's PKCE verifier and nonce are stored as they are (the
  server needs them at the callback; 10 minutes, single use; `Hidden` in `Debug`).

### Identity providers

- **Configuration** (admin API, `auth_providers`): durable id `[a-z0-9-]{2,32}` (in the callback
  URL, referenced by identities, never reused: deleting is soft), `kind` (immutable), display
  name, icon (preset key or `generic`), issuer / tenant, client id, encrypted client secret,
  scopes, groups claim (default `groups`), admin groups, allowed groups, new accounts (`auto` |
  `approval`, default `approval`: see "Account approval"), enabled, position.
  Presets (`idp/presets.rs`): OIDC `google` (fixed issuer), `microsoft` (single tenant:
  `https://login.microsoftonline.com/{tenant}/v2.0`; `common`, `organizations`, `consumers`
  refused because the issuer can't be validated), `gitlab` (gitlab.com, issuer overridable),
  `authentik`, `keycloak`, `authelia`, `zitadel`, `pocketid`, `oidc` (issuer required); OAuth2
  `github` (scopes `read:user user:email`; subject = numeric id; email = the primary **and**
  verified one of `/user/emails`) and `discord` (scopes `identify email`; `/users/@me`, email +
  `verified`). Groups only for OIDC kinds other than Google. Apple is not offered (its client
  secret is an expiring signed JWT: future work). Validation rules: `idp/fields.rs`.
- **Callback URL** = `<effective public URL>/api/v1/auth/oauth/<id>/callback`; effective public
  URL = `PUBLIC_URL` if set, else the `public_url` setting. Without one, start fails with
  `public_url_required`.
- **Outgoing HTTP** (`idp/http.rs`): one reqwest client (rustls), 5 s connect / 15 s total,
  redirects never followed (SSRF), bodies capped at 1 MiB, `User-Agent: cloudit-server/<v>`.
  Discovery documents + JWKS are cached in memory per issuer for 1 h (a cache: any instance can
  refetch); an ID token with an unknown key id refetches the JWKS once.
- **Start** (`POST /auth/oauth/{provider}/start`, POST so it is CSRF-protected: a cross-site
  GET could start a `link` flow in the victim's session): validates `return_to` (path starting
  with `/`, not `//` or `/`, no backslash or control characters, ≤ 1024 bytes; default `/`),
  stores an `oauth_flows` row (10 min) with SHA-256 of a random `state` and of the browser
  binding, the PKCE S256 verifier, a nonce, purpose (`login`, `link` with the session's user,
  `onboarding_link` with the pending signup), return path and the invite token's hash; sets
  `cloudit_oauth` (random, reused if the browser already has one so tabs don't clash; `HttpOnly;
  SameSite=Lax; Path=/api/v1/auth; Max-Age=600`) and answers the authorization URL
  (`response_type=code`, client id, redirect URI, scopes, state, `code_challenge` S256, OIDC
  `nonce`).
- **Callback**: one conditional UPDATE consumes the flow (`state_hash`, `provider_id`,
  `browser_binding_hash` = SHA-256 of the cookie, unused, unexpired), so it completes once, in
  the starting browser only (login-CSRF protection); a mismatch doesn't burn it. Provider
  `error=access_denied` → `oauth_cancelled`. The code is exchanged (HTTP Basic with
  form-encoded credentials, or `client_secret_post` when an OIDC provider lists only that;
  GitHub and Discord: in the body). OIDC: `openidconnect` verifies the ID token (signature
  against the JWKS, RS/PS/ES/EdDSA or HS* with the client secret, never `none`; issuer = the
  configured one exactly; audience = client id; expiry; nonce). Email from the token
  (`email_verified` missing = false; string booleans accepted); groups from the configured
  claim of the verified payload (array of strings, other elements ignored, or a single string;
  other types = none). If the email is missing, or the groups are while the provider uses
  groups, the user-info endpoint fills them (its `sub` must match). Then `allowed_groups`
  (non-empty and no membership → `group_not_allowed`) and the outcome (`oauth/complete.rs`).
- **Outcomes**: `login` + known identity → enabled account required, identity email and
  `last_login_at` updated, session. `login` + unknown identity → **auto-link** only if the
  `autolink_email` setting is on, the provider says the email is verified, an account has that
  email and that account's `email_verified` is true (unverified emails are never trusted for
  linking); otherwise a `pending_signups` row (15 min; token in `cloudit_onboarding`, same
  attributes, `Max-Age=900`) and a redirect to `/onboarding`. `link` → the session must still be
  the flow's user (else `oauth_state_invalid`); another account's identity → `identity_in_use`;
  the same account's → no-op. `onboarding_link` → the provider identity must already belong to
  an account (`identity_not_linked`); the pending signup is consumed first, then the pending
  identity is inserted for that account (`identity_in_use` if linked meanwhile), session.
- **Group-derived admin**: after every sign-in through an **OIDC** provider
  `admin_from_groups = admin_groups non-empty AND the identity is in one of them`, so the flag
  reflects the last OIDC sign-in (GitHub/Discord sign-ins and linking leave it unchanged).
  Clearing it is skipped (logged) when it would leave no enabled effective admin (same lock as
  `last_admin`). Accounts created at onboarding get it from the pending signup's groups.
  Such an account is also approved at creation, whatever the provider's `new_accounts`.
- **Onboarding** (cookie `cloudit_onboarding`): `create` honours the registration mode
  (`invite` → an invite from the body or carried from the
  start, consumed atomically, a bound email must match; `open` → none needed); email = typed or the provider's
  (required if none); verified only when it is the provider's verified email or an invite bound
  to it; password optional (8..=1024); `email_taken` keeps the pending signup so the user can
  link instead. One transaction: consume pending signup (first statement), user, identity,
  invite. `link` verifies email + password exactly like login, then attaches the identity.
  `link/{provider}/start` starts an `onboarding_link` flow. Success sets the session cookie and
  clears the onboarding cookie; a pending new account (see "Account approval") gets 202 and
  only the cleared onboarding cookie.
- **Unlinking** (`DELETE /me/identities/{id}`): one transaction whose first statement writes the
  user's row (serializes concurrent unlinks of one account), deletes the identity, then refuses
  with `last_login_method` (rolled back) if the account has no password and no remaining
  identity on an enabled, non-deleted provider.
- **Secret key** (`crypto/`): client secrets are encrypted with XChaCha20-Poly1305 (`SecretBox`:
  `[version 1][24-byte random nonce][ciphertext + tag]`, associated data
  `auth_providers:<id>:client_secret`, so a ciphertext can't be moved to another row). Key:
  `SECRET_KEY` (base64 of exactly 32 bytes); unset with SQLite → `<DATA_DIR>/secret.key`
  generated on first start (`create_new`, 0600 on Unix); unset with PostgreSQL → startup
  refused (instances must share it). At startup the setting `secret_key_check` (an encrypted
  known value) is inserted if absent and decrypted: failure → "SECRET_KEY changed or wrong",
  startup refused. `cloudit-server gen-secret-key` prints a new key.

### Scopes, folders, sharing and the trash

- **Model**: user > scope (owned by `owner_id`) > folder (synced unit) > snapshots.
  Roles, ordered: `viewer` (read scope, folders, members, history; download) < `editor`
  (+ create and rename folders; upload, commit, restore, pin, label, resolve conflicts) < `admin` (+ trash and
  restore folders, change a folder's retention, delete trashed folders for good) < `owner`
  (+ rename, trash, restore and delete for good the scope; add members, change their roles,
  remove them). Membership
  management is **owner-only**; any member can leave (`DELETE` its own membership); the
  owner can't leave. Server admins get no implicit access.
- **Access** (`scopes/access.rs`): every scope or folder handler starts with
  `ScopeAccess::load` / `FolderAccess::load` (404 when the caller isn't owner or member, or the
  scope / folder is in the trash, or the folder's scope is) and then `require(role)` (403).
  Trashed items are reached only through `load_trashed`: a scope by its owner, a folder by
  the owner or an admin of its (non-trashed) scope; anyone else gets 404.
- **Device tokens** may list and read scopes, members and folders, create scopes and create
  folders (the desktop client links local folders), and use every sync endpoint (chunks,
  snapshots, ZIP, `/me/usage`, `/events`, `/sync/heads`); renaming, folder changes, the
  trash, restores and member management need a web session.
- **Sharing** by exact email (normalized like logins): the account must exist, be enabled
  and approved, else `user_not_found` (unknown, pending, rejected and disabled look the
  same; the small enumeration for signed-in owners is accepted and rate limited). The
  owner's own email → `validation_failed`. `POST` on an existing member changes its role
  (200; new member 201); `PATCH` changes a role too. Accounts that become disabled,
  rejected or pending keep their memberships but authentication refuses them.
- **Names**: trimmed, 1..=100 characters, no control characters; unique among non-trashed
  siblings ignoring case (Unicode lowercase, no other normalization). Enforced by the
  database: `name_key` = lowercase name, `active_key` = `name_key` or NULL in the trash, UNIQUE
  `(owner_id, active_key)` / `(scope_id, active_key)` (NULLs never collide on either backend).
  Create, rename and restore are single statements; a unique violation → 409 `name_taken`,
  so concurrent creations have exactly one winner.
- **Trash**: `DELETE` sets `deleted_at`, `deleted_by` and clears `active_key` in one
  conditional UPDATE (`… AND deleted_at IS NULL`); restore is the reverse (`… AND deleted_at
  IS NOT NULL`), optionally with a new `name` (the old one may have been taken: the request
  then fails with `name_taken` and the client retries with a name). A folder of a trashed
  scope keeps its own state, is unreachable and not listed in the trash; restoring the scope
  brings it back (a folder trashed before stays trashed). `purge_at` = `deleted_at` +
  `trash_retention_days`, when the `trash_purge` job deletes it for good. The permanent
  delete endpoints set `purge_requested_at` (conditional UPDATE on trashed, unmarked
  items): a marked item is hidden from `GET /trash`, `load_trashed` and restore at once,
  and purged at the next `trash_purge` run (requested immediately).
- **Retention** (`retention_max_versions`, `retention_max_per_day` per folder: NULL =
  server default, 0 = unlimited, at most 100000; server defaults in `settings`, 20 and 5;
  folder responses show the override and the effective value). The `retention` job
  ("Background jobs") keeps, per UTC day, the `max_per_day` most recent candidates, then
  of those the `max_versions` most recent; the head, pinned and unresolved conflict
  snapshots are never deleted and don't count.

### Sync protocol

- **Objects** (blob storage, dedup per scope): `scopes/{scope_id}/chunks/{chunk_hash}` (stored
  chunk encoding) and `scopes/{scope_id}/manifests/{manifest_hash}` (zstd canonical JSON). A
  `chunks` / `manifests` row exists only for a stored object verified with `cloudit-core`;
  object first, row second.
- **Client flow**:
  1. scan the folder, chunk every file (`chunk_reader`), build the `Manifest` (empty dirs as
     `dir` entries) and its stored form (`Manifest::encode`); unchanged against the last
     synced manifest → nothing to do;
  2. `POST /folders/{id}/chunks/check` with every distinct `{hash, size}` (batches of at most
     10 000) → the missing ones, each with a `Transfer`;
  3. upload each missing chunk's `encode_chunk` bytes with the transfer: `proxy` = `PUT` to
     the server with the usual credentials; `presigned` = `PUT` to S3 **without** cloudit
     credentials (verified at commit). URLs are absolute when the server knows its public URL,
     else paths (`/api/v1/…`) to resolve against the server URL;
  4. `POST /folders/{id}/snapshots?parent=<last synced head | none>` with the stored manifest:
     201 → the new head; 200 `unchanged` → the head already is this state;
     `chunk_missing` / `chunk_invalid` → upload the listed chunks again and retry;
  5. 409 `conflict` (`head_snapshot_id` = the current head): download the head; if the user
     must decide, commit the local state with `kind=conflict` (`parent` = the old base; never
     moves the head) and later either `POST /snapshots/{conflict}/resolve` (keep server) or
     `POST /snapshots/{conflict}/restore` with `expected_head` (keep local: a `restore` copy
     becomes the head and the conflict is resolved);
  6. download a snapshot: `GET /snapshots/{id}/manifest` (decode with `decode_manifest`;
     `ETag` for caching), `POST /folders/{id}/chunks/download` for the chunks not held
     locally, fetch each `Transfer`, `decode_chunk` with the expected hash, reassemble files.
- **Commit** (server): decode + validate the manifest (bounded, blocking pool); kind `normal`
  with `parent` = head and the same manifest hash → 200 unchanged; every referenced chunk must
  have a row, else its object (presigned upload) is verified with a bounded concurrency of 8
  (HEAD, size ≤ `MAX_STORED_CHUNK_BYTES`, GET, decode) and adopted (row + usage), or deleted
  if invalid (unless a row appeared meanwhile); sizes must match the rows; the manifest object
  + row are stored if new; then one transaction: snapshot, `snapshot_chunks` (batches of 1000),
  conditional head update (`normal`). Author and device (device token) are recorded.
- **Restore**: a `restore` snapshot with the source's manifest (`snapshot_chunks` copied by
  one `INSERT … SELECT`), parent = the current head, conditional head update; stores nothing.
- **Usage and quotas**: `scopes.stored_bytes` = stored sizes of the scope's chunk and manifest
  rows, incremented in the transaction that inserts the row (only if inserted: concurrent
  duplicates count once). A user's usage = the sum over the scopes it owns (trashed
  included): members' uploads count against the owner. Effective quota = `users.quota_bytes`,
  else the `default_quota_bytes` setting, else unlimited. Estimates fail early (`check`:
  usage + uncompressed sizes of the missing chunks; uploads: usage + the stored size before
  storing the object); the charge itself is exact: with a quota, `insert_charged` first
  writes the owner's `users` row (serializing that owner's charges), inserts, increments and
  rolls back if the owner's total exceeds the quota. Space is freed only by `gc` (and a
  scope purge), which decrements the counter in the transaction that deletes the rows.
- **Objects and the collectors** (`sync/objects.rs`, the normative argument is in its
  docs): every transaction that inserts a `chunks` / `manifests` row, and every one that
  deletes objects, starts with the **scope lock** `UPDATE scopes SET stored_bytes =
  stored_bytes WHERE id = ?`. Under it a writer checks the object is in place (`HEAD`; a
  proxy upload or manifest stores it again if gone, an adoption reports the chunk
  missing) before inserting; a collector checks there is no row (and, for orphans, that
  the object is still older than the grace period) before deleting. So a deletion and an
  insertion for one key are serialized, and a row never exists without its object. GC
  deletes rows first (with a tombstone in `object_deletions` and the usage decrement, one
  transaction), then the objects of its tombstones under the lock; a scope purge deletes
  the scope row with all its rows, then the objects by prefix (writers then find no scope
  to lock). Cost: one `HEAD` per new row inside a transaction holding the scope lock (row
  inserts of one scope are serialized; puts and verification stay outside), collectors
  hold it for at most 100 object deletes (16 in flight). Commits that lose a row to GC
  answer `chunk_missing` (the manifest is stored again once); retention and restore lock
  the snapshot row first.

### Real-time events

- `GET /api/v1/events` (session or device token) is a Server-Sent Events stream: `event:`
  = the `ServerEvent` name, `data:` = its JSON payload. First `hello {server_time,
  instance_id, realtime: shared | local}`, then `folder_head {scope_id, folder_id,
  snapshot_id}` (commit, restore), `conflict {scope_id, folder_id, snapshot_id}`,
  `history_changed {scope_id, folder_id}` (pin, label, resolve, a restored conflict,
  retention deletions), `folder_changed {scope_id, folder_id, change}` and `scope_changed
  {scope_id, change}` (`created | updated | trashed | restored | deleted`),
  `membership_changed {scope_id}` (add, role change, remove, leave), `usage_changed {}`
  (to an owner after GC or a purge freed space). A `: keep-alive` comment every 25 s; a
  new `hello` when `realtime` changes. Header `X-Accel-Buffering: no`.
- **Recipients** are computed when publishing, after the change committed: the scope's
  owner and members (read before deleting, for a purge), plus the affected user of a
  membership change. A stream forwards only deliveries addressed to its user.
- **Transport** (`events/`): an in-process `broadcast` channel (1024 deep; a stream that
  lags is closed). With `REDIS_URL` each event is also queued to one publisher task
  (`PUBLISH cloudit:events {origin, recipients, event}`, order kept, failures dropped)
  and every instance holds one dedicated subscription (pinged every 15 s, reconnecting
  with 1–30 s backoff) that delivers other origins' events locally. `realtime` is
  `shared` while the subscription is up and Redis is not in backoff, else `local`.
- **Stream end**: at every keep-alive the stream re-checks its session (exists, not
  expired) or device (not revoked, not expired) and the user (enabled, approved), and
  ends otherwise. At most 20 streams per user and instance (in memory) → 429.
- **Client contract**: on every (re)connect fetch `GET /sync/heads`, then apply events;
  with `realtime: local` also poll `/sync/heads` (e.g. every 5 minutes, `If-None-Match`);
  a stream that ends means reconnect and resync.

### Background jobs

- **Scheduler** (`jobs/scheduler.rs`, started by `main` unless `JOBS_ENABLED=false`): every
  15 s (+ up to 5 s jitter, or at once when woken by a local run request) it reads the
  `jobs` rows; a job is due when `requested_at` is set, it never ran, or its interval
  passed since `last_started_at` (shared by all instances). A due job runs only after
  taking the lease `job:<name>` (`db::lease`, TTL 5 min, renewed every minute); the
  holder re-checks that it is due, records the start (`last_started_at`, request cleared,
  older `running` runs → `interrupted`, a new `job_runs` row), runs it and records outcome,
  counts and error. A lost lease stops the run at its next batch (`interrupted`). Jobs work
  in small re-checked transactions, so a crash or a second run is harmless.
- **Intervals** (constants): `retention` 15 min, `gc` 1 h, `orphan_sweep` 24 h,
  `trash_purge` 1 h, `cleanup` 1 h.
- **retention**: see "Scopes, folders, sharing and the trash"; folders in the trash or of a
  trashed scope are skipped. Selection is a pure function (`retention::select`). Each
  deletion is a transaction whose first statement re-checks and locks the snapshot (not
  pinned, not an unresolved conflict, not a head), then deletes its `snapshot_chunks` and
  the row. `parent_id` / `restored_from` of other snapshots may then name a deleted
  snapshot (no foreign key: dangling ids). `history_changed` per folder.
- **gc**: per scope, unreferenced `chunks` (no `snapshot_chunks`) and `manifests` (no
  `snapshots`) created before `now - GC_GRACE`, in batches of 500: scope lock, `DELETE …
  RETURNING` with the same conditions, tombstones, `stored_bytes -=` the deleted sizes;
  then the tombstones' objects (see "Sync protocol"). A batch failing on a foreign key (a
  racing commit, PostgreSQL) is skipped until the next run. `usage_changed` to owners.
- **orphan_sweep**: lists `scopes/`, keeps keys of the sync layout last modified before
  `now - GC_GRACE` without a row, and deletes them under the scope lock after re-checking
  (no row, `HEAD` still old); scopes that no longer exist need no lock. Other keys are
  never touched.
- **trash_purge**: scopes, then folders, with `deleted_at` before `now -
  trash_retention_days` or `purge_requested_at` set. Scope: one transaction (first
  statement re-checks and locks the scope row) deleting its `snapshot_chunks`,
  `snapshots`, `folders`, `scope_members`, `chunks`, `manifests`, `object_deletions` and
  the scope, then its objects by listing `scopes/{id}/`; `scope_changed deleted` to the
  former members, usage of the owner drops by the scope's bytes. Folder: one transaction
  (locks the folder row) deleting its snapshots' `snapshot_chunks`, its snapshots and the
  folder; its chunks and manifests are left to `gc` (they may be shared with other
  folders of the scope) and a `gc` run is requested. `folder_changed deleted`.
- **cleanup**: expired sessions, device codes and OAuth flows; expired pending sign-ups no
  flow references; used or expired password resets; unused invites that expired or were
  revoked more than 30 days ago (**used invites are kept**: they record who invited whom);
  finished job runs older than 30 days. Devices are never deleted (snapshots reference
  them).
- **ZIP**: streamed from a task through a 256 KiB pipe, files deflated with data descriptors
  (ZIP64-capable), chunks prefetched 4 at a time and hash-verified, DOS mtimes (UTC, 2 s),
  Unix modes 0644 / 0755, empty dirs as `dir/`; a failure aborts the body (no complete-looking
  truncated archive).

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
    admin, admin_from_groups, disabled_at?, approval_status (`approved` | `pending` | `rejected`,
    default `approved`), approval_decided_at?, approval_decided_by? (an admin; no FK, users are
    never deleted), signup_source? (`password` | `invite` | provider id), created_at,
    updated_at)`;
  - `sessions(id, token_hash UNIQUE, user_id → users, created_at, last_seen_at, expires_at,
    user_agent?)` (index on user_id; user agent truncated to 256 characters);
  - `setup_state(id = 1 PK, token?, created_at, completed_at?)`: one row, never deleted;
  - `settings(key PK, value (JSON text), updated_at, updated_by? → users)`; keys
    `registration_mode` = `"open" | "invite"` (missing = `invite`),
    `password_new_accounts` = `"auto" | "approval"` (missing = `approval`), `autolink_email`
    (bool, missing = false), `public_url` (string or null), `secret_key_check` (base64 of the
    encrypted check value), `retention_max_versions` (number, missing = 20),
    `retention_max_per_day` (missing = 5), `trash_retention_days` (missing = 30),
    `default_quota_bytes` (number or null, missing = unlimited);
  - `invites(id, token_hash UNIQUE, created_by → users, email?, created_at, expires_at, used_at?,
    used_by? → users, revoked_at?)`;
  - `password_resets(id, token_hash UNIQUE, user_id → users, created_by? → users (null = CLI),
    created_at, expires_at (24 h), used_at?)`;
  - `devices(id, user_id → users, name, os, token_hash UNIQUE, created_at, last_seen_at?,
    expires_at? (NULL = forever), revoked_at?)` (index on user_id);
  - `device_auth_codes(id, code_hash UNIQUE, user_id → users, code_challenge, redirect_uri,
    device_name, os, token_ttl_secs? (NULL = forever), created_at, expires_at (2 min), used_at?)`;
  - `auth_providers(id TEXT PK (slug), kind, display_name, icon, issuer_url?, tenant?,
    client_id, client_secret_enc BLOB, scopes, groups_claim?, admin_groups (JSON array),
    allowed_groups (JSON array), new_accounts (`auto` | `approval`, default `approval`), enabled,
    position, created_at, updated_at, deleted_at?)`;
  - `identities(id, user_id → users, provider_id → auth_providers, subject, email?,
    email_verified, created_at, last_login_at?)` (UNIQUE (provider_id, subject), index on
    user_id);
  - `pending_signups(id, token_hash UNIQUE, provider_id → auth_providers, subject, email?,
    email_verified, display_name, groups (JSON array), invite_token_hash?, created_at,
    expires_at (15 min), used_at?)`;
  - `oauth_flows(id, state_hash UNIQUE, browser_binding_hash, provider_id → auth_providers,
    purpose (login | link | onboarding_link), user_id? → users, pending_signup_id? →
    pending_signups, pkce_verifier, nonce, return_to, invite_token_hash?, created_at,
    expires_at (10 min), used_at?)`;
  - `scopes(id, owner_id → users, name, name_key, active_key?, created_at, updated_at,
    deleted_at?, deleted_by? → users)` (UNIQUE (owner_id, active_key));
  - `scope_members(scope_id → scopes, user_id → users, role (`viewer` | `editor` | `admin`),
    added_by → users, created_at, updated_at)` (PK (scope_id, user_id), index on user_id;
    removing a member deletes the row);
  - `folders(id, scope_id → scopes, name, name_key, active_key?, head_snapshot_id? (no FK:
    SQLite can't add one to an existing table; only commits and restores set it),
    retention_max_versions?, retention_max_per_day? (integer), emoji? (text), created_by → users,
    created_at, updated_at, deleted_at?, deleted_by? → users)` (UNIQUE (scope_id, active_key));
  - `scopes.stored_bytes` (bigint, default 0: the usage counter) and `users.quota_bytes?`
    (bigint; NULL = the server default);
  - `chunks(scope_id → scopes, hash (hex text), size (int, uncompressed), stored_size
    (bigint), created_at)` (PK (scope_id, hash));
  - `manifests(scope_id → scopes, hash, stored_size, created_at)` (PK (scope_id, hash));
  - `snapshots(id, folder_id → folders, scope_id, parent_id? (no FK), kind (`normal` |
    `conflict` | `restore`), manifest_hash, file_count, logical_size (bigint), author_id →
    users, device_id? → devices, created_at, pinned (default false), label?, restored_from?
    (no FK), conflict_resolved_at?)` ((scope_id, manifest_hash) → manifests; indexes
    (folder_id, id) and (scope_id, manifest_hash));
  - `snapshot_chunks(snapshot_id → snapshots, scope_id, hash)` (PK (snapshot_id, hash);
    (scope_id, hash) → chunks; index (scope_id, hash)): the distinct chunks of each snapshot.
  - `scopes.purge_requested_at?`, `folders.purge_requested_at?` (permanent delete requested);
  - `jobs(name TEXT PK, requested_at?, last_started_at?)` (one row per job, inserted when
    missing);
  - `job_runs(id, job, instance_id, started_at, finished_at?, outcome (`running` |
    `succeeded` | `failed` | `interrupted`), counts (JSON object text), error?)` (index
    (job, id));
  - `object_deletions(scope_id, kind (`chunk` | `manifest`), hash, created_at)` (PK all
    three but created_at; no foreign key): GC tombstones.

  Expired sessions, codes, flows, pending sign-ups, used resets and old unused invites are
  deleted by the `cleanup` job.
- **Migrations** run at startup (`db/migrate.rs`). SQLite: if any are pending and the database
  isn't new, `VACUUM INTO <DATA_DIR>/backups/cloudit-<UTC YYYYMMDDTHHMMSSZ>-pre-migrate.db`
  first; a failed backup aborts startup; backups are never pruned automatically. PostgreSQL:
  under `pg_advisory_lock(0x636c6f7564697400)`, so concurrent instances migrate once; no
  automatic backup (the operator runs `pg_dump`). Unknown applied migrations (a downgrade) abort.
- **Leases** (`db/lease.rs`): `INSERT … ON CONFLICT (name) DO UPDATE … WHERE holder = me OR
  expires_at < now`, success = 1 row affected; `renew` fails once expired. Holder = the random
  per-process `instance_id`. Instance clocks must be in sync. Redis is deliberately not used for
  leases (one mechanism, always available).
- **Blobs** (`object_store` 0.14, `STORAGE`; key layout in "Sync protocol"): `fs` = `LocalFileSystem` at `STORAGE_FS_DIR`
  (atomic writes, fsync on); `s3` = `AmazonS3` (path-style by default, `http` endpoints
  allowed). Keys must be canonical object paths. Presigned URLs only with `S3_PRESIGN=true`;
  when `S3_PUBLIC_ENDPOINT` is set, a second signer-only `AmazonS3` built with it signs the
  URLs (SigV4 signs the host), so they work for clients.
- **Redis** (`REDIS_URL`, optional): Redis or Valkey via the `redis` crate's lazy
  `ConnectionManager`. Used by the rate limiter (keys `cloudit:rl:<key>:<window>`; any error or
  2 s timeout degrades to the in-memory path and skips Redis for 5 s) and by the events
  (pub/sub channel `cloudit:events`, one dedicated subscription per instance; see
  "Real-time events"). Leases and jobs never use Redis.

### Configuration

Read once at startup (`config/mod.rs`); empty values count as unset. Documented in `.env.example`.

| Env var | Default | Meaning |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0` | IP address to listen on |
| `PORT` | `8080` | HTTP port |
| `PUBLIC_URL` | unset | External base URL (`http(s)://…`, trailing `/` trimmed): the origin for CSRF checks, `Secure` cookie when https; with the stored `public_url` setting as fallback (env wins): provider callback URLs and the URL printed by `admin reset-password` |
| `SECRET_KEY` | unset | Base64 of exactly 32 bytes (secret): encrypts provider client secrets. Unset: `<DATA_DIR>/secret.key`, generated on first start (SQLite only); required with PostgreSQL. `cloudit-server gen-secret-key` prints one |
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
| `JOBS_ENABLED` | `true` | Run the background jobs scheduler on this instance (jobs stay lease-guarded: one instance at a time) |
| `GC_GRACE` | `24h` | Age (at least `1h`) before unreferenced chunk / manifest rows and objects without row may be deleted |
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
4096 bytes; a segment over 255 bytes; leading `/` or ``, drive prefix (`C:`, `C:x`), UNC; any
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
first-run token), `admin reset-password <email>` (prints a 24-hour reset path, plus the URL when
a public URL is known; invalidates the user's older unused resets), `gen-secret-key` (prints a
new `SECRET_KEY`; needs no configuration), `healthcheck`. They use the server's configuration
and database but never migrate or serve.

## Planned design (NOT implemented yet)

Decided design for the next milestones, kept here so they don't have to re-derive it. Move each part
into the sections above as it gets built.

Milestones: 1 scaffolding (done) · 2 core chunking/manifest (done) · 3a server infrastructure
(DB, storage, Redis, readiness: done) · 3b password accounts, sessions, devices, admin API (done) ·
3c identity providers (OIDC / OAuth2), onboarding, identity linking (done) ·
4a scopes, folders, sharing, trash, retention settings (done) · 4b sync protocol, quotas
(done) · 4c events and background jobs (done) · 5 (next) web UI (from mockups) · 6 desktop manual
sync + conflicts · 7 desktop auto sync/tray/autostart/installers.

Planned layout additions: `apps/desktop` (Tauri 2 + Svelte 5, milestone 6; OS code in a `platform/`
module).

### Storage and deployment

Database, blob storage, Redis (rate limits, event pub/sub), the HA compose, the blob
transfer of the sync API and the lease-guarded background jobs are implemented.

### Desktop client

- Sync flow: "Sync protocol". Triggers: manual, startup, quiet period, associated process
  exit, SSE events. Local state in SQLite.
