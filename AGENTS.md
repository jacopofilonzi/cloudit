# AGENTS.md

Guidelines for AI coding agents (and people) working in this repository.
What lives where and how the pieces fit together is in [STRUCTURE.md](STRUCTURE.md): read it first,
instead of re-reading the whole repo.

These guidelines come from a shared template: https://github.com/jacopofilonzi/agents-guideline.
See "Updating these guidelines" below to align them with the latest version.

## What it is

cloudit (provisional name): self-hosted client-server folder sync (e.g. game saves) with per-user
scopes, versioned snapshots and a desktop client. Stack: Rust (axum, SeaORM, object_store) server;
Rust sync engine for the desktop (`cloudit-client`: rusqlite, reqwest, keyring-core);
Tauri 2 + SvelteKit/Svelte 5 + Tailwind 4 desktop and web; pnpm + Cargo workspaces. Targets: Linux
server (Docker); desktop Windows + Linux (macOS later).

## Workflow

- **Non-trivial changes start with a plan.** For a feature, refactor, new dependency or anything touching
  several files: read what you need, then present the plan (what changes, which files, trade-offs, open
  questions) and wait for my approval before editing. Small, obvious fixes can go straight in. If the plan
  changes significantly along the way, stop and ask again.
- **Big changes go to one agent, small ones you do yourself.** The goal is to save the main conversation's
  context: the agent does the heavy reading and editing in its own context. Don't spawn sub-agents for
  everything. For a big change, once I've approved the plan, write it out in full (goal, steps, files it
  owns, constraints, how to verify) and launch a single agent to carry it out following that plan; it
  answers with a short report (what changed, which files, `make check` result, open issues), not file
  dumps. Then check it from the diff and `make check` instead of re-reading everything it read. Small
  changes you make directly in the main conversation, without agents.
- **Propose an architecture change when something outgrows its start.** A part that began small and
  simple (e.g. a dashboard in plain HTML for a couple of tasks, a single-file script, a hand-rolled
  config) may keep gaining responsibilities. When it starts to strain its original approach (state wired
  by hand, duplicated markup or logic, ever-growing glue code), stop and propose moving to a better fit
  (e.g. a framework, a proper module layout), with trade-offs and a migration path, instead of piling more
  onto the simple version. Don't migrate without my approval.
- When something is genuinely my decision (UX, user-visible naming, data formats, breaking changes), ask
  instead of guessing. Otherwise pick the sensible default and say what you picked.
- **Flag breaking changes.** The app and the server are released separately and installed apps
  lag behind: the desktop app should stay compatible with the previous versions of the server
  and the server with the previous versions of the app. Breaking changes are allowed, but
  whenever a change breaks compatibility with the previous release (API, `/health` levels,
  sync formats, local or server data, config, update channel), tell me explicitly in the plan
  and again in the final report: what breaks, for which versions, and the migration path.
- Run commands from the repo root through the Makefile (`make help` lists them). A new recurring command
  goes into the Makefile too.
- **While developing, use the framework's dev tools** (`make dev`: dev server, hot reload, `wails3 dev`, watch
  mode…) instead of building and launching the app after every change. Build and run the real artifact
  (binary, installer, Docker image) only when I say the dev tools don't pick up a specific change, and only
  for that check; then go back to the dev server.
- **Always run `make check` before declaring work done.** It must pass with zero errors and zero warnings.
  If you couldn't run something (Docker, a live API, a platform you're not on), say so instead of claiming it works.
- Makefile recipes must work in both `sh` and `cmd.exe` (GNU make on Windows falls back to `cmd.exe`): only
  `cd dir && command`, no inline `VAR=x cmd`, no `rm`/`cp`/`mkdir -p`; use `node -e` (or the project's own
  language) for file operations.
- Every new env var goes in the config module **and** is documented in `.env.example` (and in
  `docker-compose.yml` if relevant for deployment). Never commit `.env` or secrets.
- **Never cause real side effects without asking**: payments, bookings, emails, messages to real users,
  writes to external services. Tests use fakes and fixtures; live checks are read-only. When running the
  app locally for a check, start from an empty data directory unless told otherwise, and stop it afterwards.
  In this project, for now: nothing acts for real besides the server writing to its configured
  database, blob storage (`DATA_DIR` or the S3 bucket) and Redis. Tests touch only temp dirs and
  the throwaway services of `make test-ha` (`TEST_*` variables): never point them at real
  ones (the tests create databases and objects there). There is no email (SMTP) sending: invites
  and password resets are links handed out by admins or the CLI. Identity providers (OIDC,
  GitHub, Discord) are contacted only by the running server, for admin create/test and user
  sign-ins; tests use the fake provider in `routes/tests/fake_idp.rs` on 127.0.0.1 and never a
  real one. For manual checks use dedicated test OAuth apps (and test buckets), never the
  production ones.
- **Development never uses production credentials or services**: use test accounts, test bots, sandbox
  keys and a separate database.

### Commits

- Small, step-by-step commits as the work progresses, never a single commit at the end. Conventional
  prefixes: `feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`.
- **Exception, experiments:** while we're trying things out (styling, layouts, alternative approaches),
  don't commit. Commit only once I've picked the version to keep.
- Mockups and design alternatives are never committed: they only serve to choose; the chosen one lives in the code.
- Push, tag and release only when I ask.

### Keep the agent docs in sync, in the same change

`STRUCTURE.md` and this file exist so agents don't have to re-read the whole repo. When you add, move,
rename or delete a file or module, or change an API endpoint, env var, invariant, data format or workflow
command, update `STRUCTURE.md` / `AGENTS.md` (and `README.md` for user-facing changes) in the same commit
or right after. Before declaring work done, check that every path, name and value they mention for the
areas you touched still matches the code.

### Updating these guidelines

When I ask you to check for new guidelines, fetch the latest template from
https://github.com/jacopofilonzi/agents-guideline (raw files:
`https://raw.githubusercontent.com/jacopofilonzi/agents-guideline/main/{AGENTS,CLAUDE,STRUCTURE}.md`)
and align this project's copies with it:

- Bring in new and changed generic rules (workflow, commits, code, testing…) and drop the ones removed
  upstream, without asking.
- Keep everything project-specific: filled-in placeholders, invariants, recipes, pitfalls, the whole
  content of `STRUCTURE.md`, and rules this project added or deliberately changed.
- Ask only when an upstream change conflicts with the project's current style or guidelines (e.g. it
  contradicts a project-specific rule or a convention the code follows); list each conflict with both versions.
- Then summarize what changed and commit it as `docs: sync agent guidelines`.

## Invariants — don't break these

- Secrets (tokens, passwords, client secrets) live only in the redacted `Secret` type
  (`crates/server/src/config/secret.rs`), or `Sensitive` in API bodies (`cloudit-core`
  protocol, `Debug` prints `***`): never logged, never returned except the one response that
  hands a new token to its owner. The only deliberate exception is the first-run setup token,
  logged at WARN while setup is pending (and stored in plaintext so the CLI can print it).
- Tokens (sessions, devices, invites, resets, device codes) are stored only as SHA-256 hashes
  (`auth::tokens::hash_token`), passwords only as argon2id hashes. Compare secrets in constant
  time (`constant_time_eq`) when they aren't looked up by hash.
- Setup and every single-use token (invite, password reset, device code) are consumed with one
  conditional UPDATE (`… WHERE used_at IS NULL AND expires_at > now`, or `completed_at IS NULL
  AND token = ?` for setup) whose affected-row count decides the winner; never read-check-write.
  The `setup_state` row is never deleted.
- Every `/api/v1` handler states what credentials it accepts through its extractor
  (`SessionUser`, `AnyUser`, `AdminUser` or none); device tokens never reach admin, device
  management, session or credential routes. Disabled accounts are checked on every request.
- State-changing browser requests pass the CSRF check (`auth::csrf`); no CORS is enabled.
- Redis is optional and never fails a request: every call goes through `cache::Redis::run`
  (timeout, backoff) and degrades (in-memory rate limit, polling instead of pub/sub); the server
  starts without it and `/ready` never fails because of it. Job leases always use the database
  (`db::lease`), never Redis.
- The server is stateless: no correctness-relevant in-memory state. HA comes from the database plus
  object storage; any instance can serve any request.
- A snapshot commit is the conditional head update
  (`UPDATE folders SET head_snapshot_id = new WHERE id = ? AND head_snapshot_id = parent` → 201,
  or 409 `conflict`), in the transaction that inserts the snapshot; never a read-modify-write.
  Restores do the same; `conflict` snapshots never move the head.
- Chunks are content-addressed per scope namespace (`scopes/{scope_id}/chunks/{sha256}`): no
  cross-scope dedup, so an editor of a shared scope can never write into the namespace of the
  owner's other scopes. Storage quotas charge a scope's stored size to the scope's owner.
- **A `chunks` / `manifests` row means a verified object**: every uploaded chunk and manifest
  is decoded with `cloudit-core` (bounded, hash verified) before its row exists; the object is
  stored first, the row second (an object without a row is an orphan for GC). Objects
  uploaded through presigned URLs get their row only when a commit verifies them.
- **Chunk packs are a wire format only, never stored** (`cloudit_core::pack`, API level 2):
  the server verifies every record of an uploaded pack before storing any, then stores
  each chunk as its own object and row through `sync::chunks::store_verified`, the same
  path as a single `PUT`; never add a second write path or store a pack as an object.
  Clients use packs only when `/health` announced `api_level >= 2` and decode every
  record against the hash they asked for.
- **The scope lock guards rows against object deletion** (`sync::objects`): every
  transaction that inserts a `chunks` / `manifests` row (`usage::insert_charged`) or deletes
  objects of a scope (GC, orphan sweep, invalid adoptions) starts with
  `objects::lock_scope`; a writer re-checks its object (`ensure_present`) and a collector
  re-checks that no row exists under that lock. Never delete a sync object or insert its
  row any other way. GC deletes rows (tombstone + usage in one transaction) before
  objects; a scope purge deletes the scope row before its objects.
- **The usage counter `scopes.stored_bytes` changes only in the transaction that inserts or
  deletes the row** (`sync::usage::insert_charged`: `INSERT … ON CONFLICT DO NOTHING`, charge
  only when a row was inserted; `jobs::gc` decrements by the sizes `DELETE … RETURNING`
  gave back). `usage::recompute` is the
  reference; tests check the two agree.
- Every request on a scope or folder resolves the caller's role through `scopes::access`
  (`ScopeAccess` / `FolderAccess`, then `require(role)`), never with ad-hoc queries in a
  handler: anything the caller can't see (not a member, in the trash, unknown id) is 404
  `not_found`; 403 `forbidden` only when it is visible but the role is too low. Server admins
  get no implicit access to users' scopes. Membership management is owner-only (any member
  may leave).
- Scope and folder names are unique (ignoring case) among non-trashed siblings through the
  `active_key` UNIQUE indexes (NULL in the trash): create, rename and restore are single
  statements that map a unique violation to `name_taken`; never check-then-insert.
- Snapshots are immutable; a restore creates a new snapshot. Deleting data is soft (trash), except
  the retention, GC and trash purge jobs (GC with a grace period, the purge after
  `trash_retention_days` or an explicit permanent delete).
- **`/api/v1` changes are additive only** (the app and the server are released separately,
  so installed apps talk to older and newer servers): new endpoints, new optional fields,
  new error codes (an app that doesn't know a code falls back on the HTTP status); never
  remove a field, endpoint or code or change its meaning, and a new value of an enum that
  older apps read in responses (snapshot kinds, roles…) counts as a change. A new
  request field is optional; a new response field that older servers don't send is
  `#[serde(default)]` where the app reads it. When the app starts relying on something new,
  bump `cloudit_core::protocol::API_LEVEL` (server) and `cloudit_client::compat::REQUIRED_API_LEVEL`
  (app): an older server then gets the "server too old" message instead of failing
  oddly. Removing or changing something requires raising the server's
  `MIN_CLIENT_VERSION` (`routes/health.rs`): a breaking server release, my decision.
  `/health` stays cheap and dependency-free (the container probes call it).
- Frontend API types are generated from Rust (ts-rs) into `packages/api-types/src/generated/`: run
  `make types` and commit the result; never hand-write API models in TS. `make check` fails if they
  are stale.
- `DEFAULT_LANG` is `en`.
- **The sync formats are durable** (stored on servers): the chunking parameters (FastCDC 2020,
  64/256/1024 KiB, level 1, seed 0), the stored chunk encoding, the manifest JSON and its
  canonical form, the hashing and the path rules (STRUCTURE.md "Sync formats", module docs in
  `crates/core`). Changing any of them requires a new format version (manifest `version`, chunk
  header byte) that still reads the old one. Golden-vector tests pin them: never edit a golden
  test to make a change pass.
- **Upgrading production never loses data or stops the service.** Durable data is not cache: IDs
  and formats stay backward compatible. Schema changes are migrations appended to
  `crates/server/src/db/migrations/`, never edits to one already released (pushed to `main`).
  Migrations use the SeaORM schema builder only (no backend-specific SQL), follow the table
  conventions in `db/mod.rs` (UUID v7, UTC timestamps) and must pass on both SQLite (`make
  test`) and PostgreSQL (`make test-ha`): run `make test-ha` before committing any change that
  touches SQL. A migration that touches existing rows gets a test that builds a database at the
  previous version with realistic data and migrates it. On SQLite the app backs up the database
  before migrating and refuses to start if the backup fails; on PostgreSQL the operator backs up
  (`pg_dump`, README upgrade notes). Anything the operator must do before deploying (env vars,
  manual steps) goes in the README upgrade notes.
- Only enabled **and approved** accounts sign in or use credentials: every authentication point
  (login, provider sign-in, linking into an account, sessions, device tokens) goes through
  `accounts::users::ensure_active`, and every new way to create an account decides its
  `approval_status` (a valid invite approves; otherwise the sign-up path's `auto`/`approval`
  policy). Rejected accounts are kept (a ban), never deleted.
- Every multi-instance coordination goes through the database (atomic conditional statements,
  leases, advisory locks), never through in-process state: any instance can die at any moment.
- **Background jobs are always lease-guarded and idempotent** (`jobs::scheduler::run_leased`):
  a job runs only while its instance holds `job:<name>`, works in small transactions that
  re-check their conditions (deleting a snapshot, folder or scope first locks its row with a
  conditional UPDATE), and checks `RunContext::check` between batches. Retention never
  deletes the head, pinned snapshots or unresolved conflicts.
- **Events are published after the change committed** (`events::notify`), to recipients
  computed then (scope owner + members, plus the affected user of a membership change):
  never to users without access, never from inside a transaction. Publishing never fails a
  request; Redis only extends delivery to other instances.
- Identity-provider client secrets are stored only encrypted (`crypto::SecretBox` with the
  server key, associated data naming the row) and never returned by the API (`has_secret`
  only). The key comes from `SECRET_KEY` or, SQLite only, `<DATA_DIR>/secret.key`; with
  PostgreSQL the server refuses to start without `SECRET_KEY`, and any instance refuses to
  start when the stored check value doesn't decrypt. Never change the stored format without a
  new version byte that still reads version 1.
- Provider sign-in starts with a POST (CSRF-checked; a GET would let another site start a
  `link` flow in the victim's session); the callback must come from the browser holding the
  `cloudit_oauth` binding cookie; flows and pending signups are single-use rows consumed with
  one conditional UPDATE. PKCE S256, `state` and (OIDC) `nonce` always.
- Never trust an unverified email for account linking: auto-link needs the setting on, the
  provider's `email_verified` and the account's own `email_verified`. The outgoing provider
  HTTP client never follows redirects.
- **The client never overwrites or deletes a local file whose current content isn't already
  stored on the server** (`crates/client`, crate docs; no local backup copies). Every file a pull
  replaces or deletes must have its chunk list in the `SafeSet` (the head, the base if
  `GET /snapshots/{base}` still finds it, this device's conflict snapshot); otherwise the local
  state is committed as a `conflict` snapshot first, and if that fails nothing is touched.
  Right before each replacement the file is re-checked against the scan (size + mtime, hashes
  when racy) and replaced by one rename of a fully staged temp file. Never add a code path that
  writes or deletes in a synced folder without going through `sync::pull` / `sync::stage`.
  Nothing but the synced files and `.cloudit-tmp-*` temp files is ever written inside a synced
  folder; engine state lives in its data dir. The device token lives only in the OS keychain
  or the 0600 fallback file (`Secret`: never in the state database or a log). The client's
  SQLite migrations (`crates/client/src/state/migrations.rs`) are append-only like the server's.

## Code

- Everything is written in English (code, comments, commits, docs), except the language packs.
- Keep modules small and focused: one folder per area; split a file when it grows several responsibilities.
- Comments explain *why*, not *what*. Put the design of a non-trivial module in its top-level doc comment
  (upstream formats, invariants, data layout).
- Never ignore errors: wrap them with context. No `unwrap()`/`expect()`/unchecked casts on external data
  (network, files, user input).
- API errors carry a stable machine-readable `code` (and an English message for logs); the UI maps the
  code to its own text. User-facing text never comes from the backend error.
- Use the project's logger, never `println!`/`console.log`/`fmt.Println` in core logic.
- OS-specific code is isolated in one place (`platform/`, `_windows`/`_linux`/`_darwin` files); no
  branching on the OS elsewhere. Build paths with the language's path join, never by concatenating `/`.
- Reuse shared clients and connections (HTTP client, DB pool) instead of creating them per request.
- **Dependencies are recent majors whose APIs differ from most online examples:** check the docs for the
  installed version before using an API. Ask before adding a new dependency.
  Notable versions in use:
  - Rust edition 2024, resolver 3. axum 0.8 (`{param}` path syntax), tower-http 0.7, ts-rs 12
    (dev-dependency: types derive `TS` under `cfg(test)`; `TS_RS_LARGE_INT = "number"`).
  - cloudit-core: fastcdc 5 (`v2020` module), sha2 0.11 (digest 0.11, `hybrid-array` outputs),
    zstd 0.14 (bundles C zstd: needs a C compiler, present in the `rust:*-slim` image), hex 0.4.
  - **SeaORM 2.0** (sea-query 1.0, sqlx 0.9; features `runtime-tokio-rustls`, `sqlx-sqlite`,
    `sqlx-postgres`, `with-chrono`, `with-uuid`): `.eq()`/`.lt()` on `Expr` need
    `sea_query::ExprTrait` in scope; `ConnectionTrait::execute(&stmt)` takes a statement builder
    (raw SQL: `execute_raw(Statement)` / `execute_unprepared`); migrations implement
    `use_transaction()`; sqlx is reachable as `sea_orm::sqlx`.
  - **object_store 0.14** (`aws` feature): `put`/`get`/`head`/`delete` live in the
    `ObjectStoreExt` trait; presigning is `signer::Signer::signed_url`.
  - **async_zip 0.0.19** (`tokio`, `deflate`): `ZipFileWriter::with_tokio`,
    `write_entry_stream` (data descriptors, ZIP64 extra field always written) + `close()`;
    custom extra fields can't be built (the header types are private); the in-memory reader
    `base::read::mem` used by tests is deprecated in favour of `read1`. tokio has the
    `io-util` feature (`duplex` pipe for streamed bodies). The server tests also use `zstd`
    (dev-dependency) to build invalid stored manifests.
  - Client (`crates/client`): **rusqlite 0.39** (`bundled`; not 0.40+, whose `libsqlite3-sys`
    clashes with sqlx 0.9's `links = "sqlite3"`: one SQLite for the workspace), walkdir 2.5,
    globset 0.4 (`GlobBuilder::literal_separator`), tokio-util 0.7
    (`CancellationToken::run_until_cancelled`), **keyring-core 1.0** with one store per OS (not
    the `keyring` 4 meta-crate): windows-native-keyring-store 1.1 (no default features),
    zbus-secret-service-keyring-store 1.0 (`crypto-rust`: pure Rust D-Bus),
    apple-native-keyring-store 1.0 (`keychain`); `Store::new()?` then
    `store.build(service, user, None)` gives an `Entry` (no global default store). std
    `File::set_modified` sets mtimes. The client tests run the real server through the
    `cloudit-server` dev-dependency.
  - **redis 1.8** (not the 2.0 release candidates; features `tokio-comp`, `connection-manager`,
    no TLS): `ConnectionManager::new_lazy_with_config`. uuid 1 (`v4`, `v7`), chrono 0.4 (the
    only date/time crate: SeaORM and object_store use it), url 2.
  - **reqwest 0.13** (`rustls` only: no `json`/`form` features, so bodies are built with
    `serde_json` / `url::form_urlencoded`; `Response::chunk` for bounded reads).
  - **openidconnect 4.1** (oauth2 5) with `default-features = false` (no bundled reqwest 0.12)
    + `accept-string-booleans`: HTTP goes through `idp::http::HttpClient::adapter` (any
    `Fn(HttpRequest) -> Future` is an `AsyncHttpClient`). `CoreProviderMetadata::discover_async`
    (checks the issuer, fetches the JWKS), `CoreIdTokenVerifier::new_confidential_client` +
    `set_allowed_algs`, `IdToken::claims(&verifier, &nonce)`; `CoreRsaPrivateSigningKey::from_pem`
    wants PKCS#1 PEM (tests). Typestate clients are not used: the code exchange is our own form
    POST.
  - **chacha20poly1305 0.11** (aead 0.6, `hybrid-array`: `XNonce::from([u8; 24])`,
    `Key::from([u8; 32])`, `Aead::encrypt(&nonce, Payload { msg, aad })`), features `alloc` only
    (nonces come from `getrandom::fill`).
  - Auth: argon2 0.6 (password-hash 0.6: `hash_password(bytes)` makes its own salt with the
    `getrandom` feature, `verify_password(bytes, &str)`), getrandom 0.4 (`getrandom::fill`),
    base64 0.23 (`Engine` + `URL_SAFE_NO_PAD`), subtle 2.6. SeaORM entities use the 2.0 format
    (`#[sea_orm::model]` + `DeriveEntityModel`, no relations).
  - TLS is rustls everywhere (no OpenSSL): sqlx uses the `ring` provider, object_store/reqwest
    `aws-lc-rs` (its C build needs a C compiler; it builds on Windows and in the Docker image).
  - **Tauri 2** (not the 3 alphas; desktop app `apps/desktop`): tauri 2.12, tauri-build 2.7,
    tauri-plugin-dialog 2.8, tauri-plugin-opener 2.7, tauri-plugin-single-instance 2.5
    (registered first: one app instance only), tauri-plugin-notification 2.8
    (`app.notification().builder().title(..).body(..).show()`), tauri-plugin-autostart 2.7
    (`Builder::new().arg(..).build()`, `app.autolaunch()`: `enable` / `disable` / `is_enabled`;
    its state exists only if the plugin's setup succeeded: `try_state::<AutoLaunchManager>()`),
    tray: tauri features `tray-icon` + `image-png` (`TrayIconBuilder`, `Image::from_bytes`,
    menus from `tauri::menu`; `&` in labels is a mnemonic: double it), **tauri-plugin-updater 2.14**
    (pinned to the minor; not the 3 alphas: `app.updater()?.check()` → `Option<Update>`,
    `update.download(on_chunk, on_finish)` then `update.install(bytes)` (verifies the signature; on
    Windows it starts the installer and exits the process), `AppHandle::restart()` on Linux; config in
    `tauri.conf.json` `plugins.updater` {pubkey, endpoints, windows.installMode}; no
    `tauri-plugin-process`), tracing-appender 0.2; JS `@tauri-apps/api` and
    `@tauri-apps/cli` 2.12. The Tauri CLI rewrites the `features = […]` lists of the desktop
    `Cargo.toml` on build (`tauri-build`: `[]`; `tauri`: `["tray-icon", "image-png"]`, kept as is):
    keep that form committed so builds don't dirty the tree.
  - Desktop automatic sync: **notify 8.2** (not the 9 release candidates; plain watcher,
    no debouncer crate: `notify::recommended_watcher(|res: notify::Result<Event>| …)` then
    `watcher.watch(path, RecursiveMode::Recursive)`; `Event::need_rescan()`) and **sysinfo
    0.40** (`default-features = false, features = ["system"]`): `System::new()` returns a
    `Result`; `refresh_processes_specifics(ProcessesToUpdate::All, true,
    ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet).without_tasks())`;
    `Process::name()` is an `&OsStr`. The desktop tests use tokio's `test-util` feature
    (`#[tokio::test(start_paused = true)]`, `tokio::time::advance`).
  - **SvelteKit 3** (not 2): no `svelte.config.js` (options go to `sveltekit({...})` in
    `vite.config.ts`); `$lib` is replaced by `#lib` (package.json `imports`, explicit extensions:
    `#lib/api.ts`); `$app/environment` → `$app/env`; `$app/stores` is gone (use `$app/state`);
    `base` from `$app/paths` is gone (use `resolve()`/`asset()`); tsconfig extends `$app/tsconfig`.
    adapter-static 4, Vite 8, vite-plugin-svelte 7, Svelte 5.57 (generic components:
    `<script lang="ts" generics="T extends string">`, not `generic`). **Typed `resolve()`**:
    route IDs keep groups (`'/(auth)/login'`), so links use base-relative pathnames without the
    leading `/` (`resolve('login')`, `resolve('')`); a union-typed or dynamic path can't be
    passed (the guard builds those from `APP_ROOT`). ESLint's
    `svelte/no-navigation-without-resolve` wants every `href` / `goto` wrapped in `resolve()`.
  - **TypeScript pinned to 6.0.x**: svelte-check, typescript-eslint and SvelteKit don't support 7 yet.
  - Tailwind 4.3 via `@tailwindcss/vite`; ESLint 10 (flat config); vitest 5; pnpm 12 (pinned in
    `packageManager`, which refuses versions younger than its minimum release age: pick an
    older version rather than adding a `minimumReleaseAgeExclude` entry).
  - **@lucide/svelte 1.x** (the Svelte 5 package, not `lucide-svelte`): import each icon from
    its own module, `import Folder from '@lucide/svelte/icons/folder'` (props `size`,
    `strokeWidth`, `fill`, `class`); the barrel import makes the dev server load every icon.
  - **simple-icons 16**: named exports `siGithub`… (`{title, slug, hex, path, svg}`), 5 MB of
    ESM that the build tree-shakes; Microsoft, ZITADEL and Pocket ID have no logo there.
- Deleting user data means moving it to the trash / soft delete, unless agreed otherwise.

### UI

- **The design is decided: mockup "B2"** (light, indigo accent `#5b5bd6`, dark values derived).
  Tokens and `@utility` patterns live in `packages/ui/src/styles.css`, shared components in
  `packages/ui/src/lib/components/` (STRUCTURE.md "Shared UI"): reuse them instead of
  restyling pages. New screens or a visibly different layout still start from a mockup the
  user approves (never committed).
- Theme: `light` / `dark` / `system` (default) through `theme.svelte.ts`, applied as
  `<html data-theme>`; style dark mode with the tokens (or the `dark:` variant), never with
  `prefers-color-scheme` directly.
- Icons are `@lucide/svelte` (UI) and `ProviderIcon` (identity-provider logos); no emoji as
  icons. Folder emoji are user data (`FolderInfo.emoji`, `EmojiPicker`).
- Destructive actions ask through `confirmAction()` / `ConfirmDialog`, never the browser's
  `confirm()` / `alert()`. Transient feedback uses `toast`.
- Forms: show `validation_failed` + `field` under the input (`formError`), other errors in a
  `Banner`; submit buttons stay enabled (native `required` validation), with `loading` while
  the request runs.
- Every UI string goes through i18n, with both the English and the Italian key (`en` is the
  default); the type check must catch a missing key. Dictionaries live in
  `packages/ui/src/lib/i18n/` (`it` is typed against the `en` keys). Italian glossary: scope →
  "raccolta" (feminine), folder → "cartella", snapshot → "versione".
- Svelte 5 runes only (`$state`, `$derived`, `$effect`, `$props`): no legacy stores or `export let`.
  Typed state is `$state<T>(…)` (an annotated `let x: T = $state(…)` narrows to the initial value).
  Don't name a variable `state` (it clashes with the rune).
- Tailwind 4: reusable classes are `@utility` in the global CSS. Colours are semantic tokens defined there
  (`bg-surface`, `text-muted`…), never raw colours in the markup.
- API models come from the backend contract (shared or generated types): never redeclare them in the UI.
  Every call goes through one API module (base URL, auth headers, error codes).
- No external scripts from CDNs unless the platform requires them; libraries are npm packages, and heavy
  ones are loaded lazily.
- Never hard-code base paths or the API prefix: build URLs from the shared base constant.
- Routing: the root layout's guard (`apps/web/src/lib/guard.ts`) decides redirects from the
  session; pages don't redirect on their own. A new page outside the app shell goes under
  `routes/(auth)/` and, if it must stay reachable signed in or out, into the guard's open list.

## Testing

- Unit tests live next to the code. Test parsing against fixtures copied from real responses, never live
  requests; live checks, if any, are a separate opt-in command (e.g. `make smoke`).
- Before committing a fixture taken from a real response, replace names, emails, phones and any token or ID
  that grants access to something on its own. Never commit real user files (documents, tickets, uploads).
- End-to-end checks: drive the built app with a headless browser (Playwright with the system Chrome/Edge;
  install `playwright-core` in a scratch dir, not in the repo), unless the project has its own e2e setup.

## Versions and releases

- Versions are `x.y.z`: `x` major (only when I decide), `y` feature, `z` fix. A feature bumps `y` and resets `z`.
- **Two independent versions**, released separately (a release of one never rebuilds the other); the
  files of each side are listed in `scripts/versions.mjs`:
  - **server**: `crates/core`, `crates/server` (each `Cargo.toml` `[package] version`; there is no
    shared `[workspace.package] version`), `apps/web`, `packages/*` and the root `package.json`;
  - **app**: `crates/client`, `apps/desktop/src-tauri` (`Cargo.toml`), `apps/desktop/package.json` and
    `apps/desktop/src-tauri/tauri.conf.json` (the installers and the updater use it).
  Change them only with `make version-server V=x.y.z` / `make version-app V=x.y.z` (=
  `node scripts/set-version.mjs server|app x.y.z`), which update that side's files and `Cargo.lock`.
- Tags and releases only when I ask. An **app release** = `make version-app V=x.y.z`, commit, push the
  tag `app-vx.y.z`: `release-app.yml` checks the tag against the app versions
  (`scripts/check-release-version.mjs`), drafts the GitHub Release, builds the installers with
  `tauri-apps/tauri-action@v1` (`uploadUpdaterJson`, `releaseId`; one OS at a time because each job
  rewrites `latest.json`) and publishes it with `--latest`. A **server release** =
  `make version-server V=x.y.z`, commit, push `server-vx.y.z`: `release-server.yml` checks the tag,
  pushes the multi-arch Docker image (tags `x.y.z`, `x.y`, `latest`, sha: no prefix) and creates a
  GitHub Release with `--latest=false`.
- **GitHub's "latest" release must always be an app release**: installed apps fetch
  `releases/latest/download/latest.json`, so a server release marked latest would break their update
  check. The server's `update_check` job lists the releases and picks the highest `server-v*` tag
  instead.
- **The updater signing key** signs every desktop update; installed apps trust only its public half
  (`plugins.updater.pubkey` in `tauri.conf.json`). The private key lives outside the repo (repository
  secret `TAURI_SIGNING_PRIVATE_KEY`, plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if it has one): never
  commit, print or log it. Losing it means installed apps can never update themselves again; changing
  it needs a release signed with the old key that ships the new public key.
- The desktop app never downloads or installs an update without the user's click, and never while a
  folder operation runs (`updater::install` holds automatic sync and waits).
- CI runs the checks and tests on every push and pull request, but builds and publishes artifacts (Docker
  images, binaries, installers) only on an `app-vX.Y.Z` / `server-vX.Y.Z` tag or a manual run, never
  on a plain push.

## Adding a field to an API model

1. Change the Rust type (`crates/core/src/protocol/<area>.rs` for shared API types) and run
   `make types`: the TS type check then points at every place to update.
2. Backend: migration (append, never edit a released one), entity (`db/entities/`), the queries
   in the owning module (`accounts/`, `devices/`…), validation (`validate.rs`), the model → API
   conversion (`routes/extract.rs`) and routes (create **and** update).
3. Frontend: every view that shows or edits the model.
4. Tests for the routes (valid value, invalid value, field missing on old rows); update `STRUCTURE.md`.
5. Compatibility (invariant "`/api/v1` changes are additive only"): the field is optional in
   requests, and the app copes with servers that don't send it (`#[serde(default)]`, or
   `Option`). If the app needs it to work, bump `API_LEVEL` and `REQUIRED_API_LEVEL`
   together. Never remove or repurpose a field instead.

## Adding an API error code

1. Add the variant to `ErrorCode` in `crates/core/src/protocol/mod.rs` (never rename or reuse a
   code) and map it in `crates/server/src/errors/mod.rs` (an `ApiError` variant, `code()` and
   `status()`).
2. `make types`: the type check then fails in `apps/web/src/lib/api.ts` (`SERVER_CODES`) and
   `apps/web/src/lib/errors.ts` until the new code is mapped.
3. Add the `error.<code>` key to both `en.ts` and `it.ts` in `packages/ui/src/lib/i18n/`.

## Known pitfalls

- **The desktop app runs as a single instance** (`tauri-plugin-single-instance`, registered first in the
  builder): nothing (engine, database, keychain, server follower, tray) may be opened before it, and no env
  var or flag may bypass it. Manual runs: `CLOUDIT_DESKTOP_DATA_DIR` for a scratch state, one instance at a
  time. Closing the window only hides it (the app stays in the tray): stop a manual run with the tray's
  Quit (or end the process) before rebuilding. The window is created hidden (`"visible": false`) and
  shown in `setup` unless started with `--minimized` (autostart).
- Desktop: Tauri 2 (not the 3 alpha). Every command needs an `allow-<name>` permission (`command_names.rs`
  + `capabilities/default.json`, checked by a test). A plain `cargo build` of `cloudit-desktop` loads the dev
  server; use `--features custom-protocol` or `make build-desktop` to embed the UI. Linux CI needs
  `libwebkit2gtk-4.1-dev librsvg2-dev libsoup-3.0-dev libgtk-3-dev libayatana-appindicator3-dev` (the
  tray; at runtime `libayatana-appindicator3-1`); bundling the AppImage also needs `patchelf` and `file`.
- Desktop bundles: `make bundle-desktop` needs no signing key (the updater packages come only with
  `--config src-tauri/tauri.updater.conf.json`, set by the release workflow, which then needs
  `TAURI_SIGNING_PRIVATE_KEY`). The NSIS installer is per user (`%LOCALAPPDATA%cloudit`, no admin):
  installing it on a dev machine leaves a Start menu entry and an uninstaller. An installed app is a
  second copy of cloudit: it shares the single-instance lock with dev runs, so stop it first. Its
  update check fails (logged at warn) until a release with `latest.json` exists.
- Desktop OS-specific bits follow the `#[path]` pattern of `auto/processes*`: `tray/theme_windows.rs`
  reads the taskbar theme with `reg.exe` (`CREATE_NO_WINDOW`; `unsafe_code` is forbidden, so no raw
  registry calls) and `tray/theme_unix.rs` is not compiled on Windows: check it on CI.
  `tauri::Env::appimage` exists only on Linux: the updater reads `$APPIMAGE` itself.
- Desktop automatic sync: every automatic run goes through `ops::run_auto` →
  `Engine::sync_with`; never sync from the scheduler any other way, and keep its decisions
  in the pure `auto::plan::Planner` (tests on paused time). The process name matching lives
  in `auto/processes_windows.rs` / `processes_unix.rs` (chosen with `#[path]`): the one not
  for this OS is not compiled locally, so check it on CI (ubuntu runs it).
- Git Bash heredocs and double-quoted `node -e` strings mangle backslashes and run backticks: write files
  with the editor tools.

- The main dev machine is Windows 11; shells are PowerShell and Git Bash. Commands and scripts must work there.
- **Git Bash rewrites env values that look like paths** (`BASE_PATH=/x` becomes `C:/Program Files/Git/x`):
  prefix the command with `MSYS2_ENV_CONV_EXCL='*'`, or set the value in `.env`.
- A running `.exe` locks its binary on Windows: stop it before rebuilding.
- `cargo` may not be on PATH in shells opened before Rust was installed: prepend it
  (PowerShell: `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`; Git Bash:
  `export PATH="$HOME/.cargo/bin:$PATH"`).
- Recipes run with `--locked`: after changing a version or dependency, refresh `Cargo.lock`
  (`set-version.mjs` does it for version bumps).
- `make check` regenerates the TS types and fails if `git status` shows changes under
  `packages/api-types/src/generated/`: commit the regenerated files (new files count too).
- ts-rs exports only from tests (`cfg_attr(test, derive(ts_rs::TS), ts(export))`); the export dir
  comes from `TS_RS_EXPORT_DIR` in `.cargo/config.toml`.
- The server binary also serves as the container probe (`cloudit-server healthcheck`), so the
  runtime image needs no curl.
- The Docker image starts as root on purpose: `docker/entrypoint.sh` chowns `DATA_DIR` to
  `PUID:PGID` (default 10001) and drops privileges with `setpriv`. Don't add `USER` to the
  Dockerfile; the server itself never runs as root.
- SQLite `sqlite::memory:` is one database per pooled connection: tests and tools use a file
  (`db::testing::TestDb`). SQLite allows one writer: keep write transactions short, hash
  passwords before `begin()`, and make the first statement of a writing transaction a write (a
  transaction that read first fails with `SQLITE_BUSY` instead of waiting).
- One request must not run many write transactions concurrently on SQLite: a chunk pack
  with 8 parallel `insert_charged` starved other writers past the 5 s busy timeout
  (`database is locked`, 500). Parallelize the object I/O, keep the rows of one request
  sequential (`routes/chunks.rs::upload_pack`).
- SQLite can't add a foreign key to an existing table (`folders.head_snapshot_id` has none;
  only the commit / restore statements set it). New tables can carry composite foreign keys
  (`snapshots → manifests`, `snapshot_chunks → chunks`).
- Raw request bodies: set a per-route `DefaultBodyLimit` and take `Result<Bytes,
  BytesRejection>` through `routes::extract::body_bytes`, so an oversized body answers JSON
  413 `payload_too_large` (axum's own rejection is plain text). `ApiJson` maps it too.
- A presigned PUT URL stays valid for `S3_PRESIGN_TTL` even after the chunk was verified: an
  editor holding one could overwrite the object. Clients and the ZIP download decode every
  chunk against its expected hash, so damage is detected, never taken as valid data; keep
  the TTL short.
- Unsafe API requests without a bearer token need `Origin` = the server's origin (or
  `Sec-Fetch-Site: same-origin`): curl and scripts must send `-H 'Origin: http://host:port'`
  (with `PUBLIC_URL` set, its origin). API route tests go through `routes::tests::support::TestApp`,
  which sends it, runs on PostgreSQL under `make test-ha`, and keeps a fresh rate limiter per test.
- While first-run setup is pending every `/api/v1` route except `/health`, `/setup` and
  `/auth/config` answers 409 `setup_required`: tests call `TestApp::setup_admin()` first.
- ts-rs ignores (and warns about) `#[serde(transparent)]`: newtypes serialize as their inner
  value anyway, so leave it out. Optional request fields are `Option<T>` with `#[serde(default)]`
  and `#[cfg_attr(test, ts(optional))]`; request structs use `#[serde(deny_unknown_fields)]`.
- Edition 2024 `impl Trait` return types capture every input lifetime: use `+ use<'a>` when a
  returned future must not borrow the other arguments.
- `TEST_DATABASE_URL` must point at a throwaway PostgreSQL server: each test creates its own
  `cloudit_test_*` database there (`make test-ha` removes the whole container afterwards).
  With PostgreSQL the server needs `SECRET_KEY`: `TestApp` sets a fixed test key then.
- Deleting or replacing `<DATA_DIR>/secret.key` (or changing `SECRET_KEY`) makes the server
  refuse to start ("SECRET_KEY changed or wrong"): restore the old key. Dev data dirs keep
  theirs; start from an empty data dir to get a new one.
- SeaORM 2 `ModelTrait` requires `Debug` on every model: a column holding a secret gets a
  `DeriveValueType` newtype with a redacting `Debug` (`entities::oauth_flows::Hidden`).
- Provider tests: `routes::tests::oauth::Env` sets up the fake provider, an admin, a public URL
  and the providers `sso` (OIDC), `gh`, `dc`; GitHub/Discord URLs can only be redirected under
  `cfg(test)` (`Idp::set_social_endpoints`). The RSA keys in `routes/tests/fixtures/` are test
  keys: never use them for anything else. The fake answers user info with `{"sub": …}` by
  default (`default_userinfo`): an ID token without email triggers a user-info call.
- Test S3 is Garage (`dxflrs/garage`, config `docker/garage.toml` with example secrets):
  `--single-node --default-bucket` creates the layout, bucket and key from `GARAGE_DEFAULT_*`
  at startup (no init job). Its region is `garage` (`S3_REGION=garage`, else signatures
  fail); key ids must be `GK` + 24 hex digits and secrets 64 hex digits. The image has no shell
  or curl: health-check it with `/garage bucket info <bucket>`. GitHub service containers can't
  take a command or mount files, so CI starts Garage with `docker run`.
- Race tests pause code at `hooks::Point`s (`hooks::arm(point, key)` under `cfg(test)`;
  keys are scope ids, or `<instance id>:<job>` for jobs, so parallel tests don't collide).
  Points sit outside transactions: a paused task must never hold a lock (SQLite would block
  every writer). Tests drive jobs with `scheduler::run_leased(state, job, true, grace)`
  (`routes::tests::jobs::run_now` uses a zero grace).
- SSE tests read the body frame by frame (`routes::tests::events::Sse`); keep-alives are
  shortened with `state.events.set_keepalive`. `tokio::spawn` of futures that borrow in
  closures (`stream::iter(&keys).map(|k| …)`) fails with "`Send` is not general enough":
  move owned values into the futures.
- The UI picks its language from the browser (`navigator.languages`) unless one was chosen:
  headless e2e runs on this machine come up in Italian unless the Playwright context sets
  `locale: 'en-GB'`. Unit tests of formatting pass the language explicitly (Italian CLDR does
  not group 4-digit numbers: `2410`, but `24.100`).
- Web pages that show server data keep it fresh through `lib/live.svelte.ts` (`onLive` inside
  `$effect`) and `coalesced` refreshes; a handler that reads `$state` before its first `await`
  inside an `$effect` must run under `untrack`, or it re-triggers itself. Dialogs are native
  `<dialog>`s: Playwright's `dialog[open]` also matches a confirm opened above another dialog
  (take `.last()`); toasts are `[aria-live=polite] p`. Browsers log expected API errors (409
  `name_taken`, 404 `user_not_found`) as console errors: they are not bugs.
- Admin pages: the browser language decides the UI language, so Playwright contexts need `locale:
  'en-US'` for English labels. `Segmented` / `RadioCards` are `sr-only` radios: click their `label`. A
  provider form in create mode closes with Back, not Cancel, and ignores Escape while the request runs
  (wait for the error banner, not for text that also appears in a hint). On Windows the server and
  `seed` take `C:/…` paths (`WEB_DIR`, `--db`; Git Bash `/c/…` paths are not understood by the exe,
  and an absolute `--db` can fail: use a relative one). After changing sign-in providers or the
  registration mode call `session.refreshConfig()`: the login page reads `session.config`, loaded
  once. A per-account quota is `quota_bytes` (`null` = server default) or `quota_unlimited`
  (never both): the quota dialog offers "Unlimited" only while the server default is limited
  (or the account already is unlimited).
- `make seed` (`crates/server/examples/seed/`) needs a local server with `S3_PRESIGN=false`; for
  back-dated history pass the server's SQLite file with `--db`.
- The Vite dev proxy must keep the browser's Host (`changeOrigin: false` in
  `apps/web/vite.config.ts`): with Vite's string shorthand Host becomes `localhost:8080`, the
  CSRF check compares it with `Origin: http://localhost:5173` and every POST fails with
  `csrf_failed`. E2E runs against the built app on :8080 don't catch this: try `make dev` too.
- Popover menus are `position: fixed` (placed from the trigger's rectangle): tables sit in an
  `overflow-x-auto` wrapper that clips absolutely positioned children.
- Client integration tests (`crates/client/tests/common`) serve the real router on
  `127.0.0.1:0` with `PUBLIC_URL` = its URL and a fixed `SETUP_TOKEN`, and sign engines in
  through the real `/devices/authorize` flow with an in-memory keychain: never touch the real
  user keychain in tests. They run on SQLite only. Race tests hook the progress callback
  (`Phase::Staging`, `Phase::Uploading`…) to change files or cancel mid-operation.
- Windows file times have 100 ns precision (FAT: 2 s): mtimes from other devices read back
  rounded. Mtimes never count as content (the three-way comparison is on chunks), and the
  file index stores the mtime read back after writing.
- The web app is an SPA (`ssr = false`, adapter-static `fallback: 'index.html'`); in production the
  Rust server serves `WEB_DIR` and falls back to `index.html`, while unknown `/api/*` paths answer a
  JSON `not_found`.
