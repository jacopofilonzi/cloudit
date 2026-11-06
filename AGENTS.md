# AGENTS.md

Guidelines for AI coding agents (and people) working in this repository.
What lives where and how the pieces fit together is in [STRUCTURE.md](STRUCTURE.md): read it first,
instead of re-reading the whole repo.

These guidelines come from a shared template: https://github.com/jacopofilonzi/agents-guideline.
See "Updating these guidelines" below to align them with the latest version.

## What it is

cloudit (provisional name): self-hosted client-server folder sync (e.g. game saves) with per-user
scopes, versioned snapshots and a desktop client. Stack: Rust (axum, SeaORM, object_store) server;
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
  storage and database (`DATA_DIR`). Coming later: OAuth providers (OIDC, GitHub, Discord) and S3
  buckets; use dedicated test OAuth apps and test buckets only, never the production ones.
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
  (`crates/server/src/config/secret.rs`): never logged, never returned, never serialized.
- Redis is optional and never fails a request: errors degrade (DB lease for jobs, in-memory rate
  limit, polling instead of pub/sub); the server starts without it.
- The server is stateless: no correctness-relevant in-memory state. HA comes from the database plus
  object storage; any instance can serve any request.
- A snapshot commit is the conditional head update
  (`UPDATE folders SET head = new WHERE id = ? AND head = parent` → 201, or 409 on conflict), never a
  read-modify-write.
- Chunks are content-addressed per owner namespace (`owners/{owner_id}/chunks/{sha256}`): no
  cross-user dedup.
- Snapshots are immutable; a restore creates a new snapshot. Deleting data is soft (trash), except
  retention/GC jobs, which run with a grace period.
- Frontend API types are generated from Rust (ts-rs) into `packages/api-types/src/generated/`: run
  `make types` and commit the result; never hand-write API models in TS. `make check` fails if they
  are stale.
- `DEFAULT_LANG` is `en`.
- **Upgrading production never loses data or stops the service.** Durable data is not cache: IDs
  and formats stay backward compatible. Schema changes are migrations appended to
  `crates/server/src/db/migrations/` (planned location, created in milestone 3), never edits to one
  already released (pushed to `main`). A migration that touches existing rows gets a test that
  builds a database at the previous version with realistic data and migrates it. The app backs up
  the database before migrating and refuses to start if the backup fails. Anything the operator must
  do before deploying (env vars, manual steps) goes in the README upgrade notes.

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
  - **SvelteKit 3** (not 2): no `svelte.config.js` (options go to `sveltekit({...})` in
    `vite.config.ts`); `$lib` is replaced by `#lib` (package.json `imports`, explicit extensions:
    `#lib/api.ts`); `$app/environment` → `$app/env`; `$app/stores` is gone (use `$app/state`);
    `base` from `$app/paths` is gone (use `resolve()`/`asset()`); tsconfig extends `$app/tsconfig`.
    adapter-static 4, Vite 8, vite-plugin-svelte 7, Svelte 5.57.
  - **TypeScript pinned to 6.0.x**: svelte-check, typescript-eslint and SvelteKit don't support 7 yet.
  - Tailwind 4.3 via `@tailwindcss/vite`; ESLint 10 (flat config); vitest 5; pnpm 12 (pinned in
    `packageManager`).
- Deleting user data means moving it to the trash / soft delete, unless agreed otherwise.

### UI

- **The UI design is not decided yet.** Palette, layout and components will be chosen from mockups
  (milestone 5); until then keep UI minimal and neutral, and the token values in
  `packages/ui/src/styles.css` are placeholders. Real UI work starts from a user-approved mockup.
- Every UI string goes through i18n, with both the English and the Italian key (`en` is the
  default); the type check must catch a missing key. Dictionaries live in
  `packages/ui/src/lib/i18n/` (`it` is typed against the `en` keys).
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

## Testing

- Unit tests live next to the code. Test parsing against fixtures copied from real responses, never live
  requests; live checks, if any, are a separate opt-in command (e.g. `make smoke`).
- Before committing a fixture taken from a real response, replace names, emails, phones and any token or ID
  that grants access to something on its own. Never commit real user files (documents, tickets, uploads).
- End-to-end checks: drive the built app with a headless browser (Playwright with the system Chrome/Edge;
  install `playwright-core` in a scratch dir, not in the repo), unless the project has its own e2e setup.

## Versions and releases

- Versions are `x.y.z`: `x` major (only when I decide), `y` feature, `z` fix. A feature bumps `y` and resets `z`.
- The version lives in `Cargo.toml` (`[workspace.package] version`, inherited by every crate, plus
  `Cargo.lock`) and in every `package.json` (root, `apps/*`, `packages/*`; later also
  `apps/desktop/src-tauri/tauri.conf.json`): change it only with `make version V=x.y.z` (=
  `node scripts/set-version.mjs x.y.z`), which updates them all.
- Tags and releases only when I ask.
- CI runs the checks and tests on every push and pull request, but builds and publishes artifacts (Docker
  images, binaries, installers) only on a `vX.Y.Z` tag or a manual run, never on a plain push.

## Adding a field to an API model

1. Change the Rust type (`crates/core/src/protocol.rs` for shared API types, or the server API
   module) and run `make types`: the TS type check then points at every place to update.
2. Backend: migration (append, never edit a released one), repository, validation schema and routes
   (create **and** update).
3. Frontend: every view that shows or edits the model.
4. Tests for the routes (valid value, invalid value, field missing on old rows); update `STRUCTURE.md`.

## Adding an API error code

1. Add the variant to `ErrorCode` in `crates/core/src/protocol.rs` (never rename or reuse a code) and
   map it in `crates/server/src/errors/mod.rs` (`code()` and `status()`).
2. `make types`: the type check then fails in `apps/web/src/lib/api.ts` (`SERVER_CODES`) and
   `apps/web/src/lib/errors.ts` until the new code is mapped.
3. Add the `error.<code>` key to both `en.ts` and `it.ts` in `packages/ui/src/lib/i18n/`.

## Known pitfalls

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
- The web app is an SPA (`ssr = false`, adapter-static `fallback: 'index.html'`); in production the
  Rust server serves `WEB_DIR` and falls back to `index.html`, while unknown `/api/*` paths answer a
  JSON `not_found`.
