# cloudit

> Provisional name.

cloudit is a self-hosted client-server folder sync service: point the desktop client at a
folder (game saves, configs, documents) and it keeps versioned snapshots of it on your own
server, synced across your devices. Folders are grouped in per-user scopes that can be shared
with other users.

**Status: early development.** The server has its storage infrastructure (database, blob
storage, optional Redis), the account API (setup, password accounts, invites, desktop device
login, admin, sign-in with OIDC / GitHub / Discord), scopes, folders, sharing and the trash
(API only), the sync protocol (chunk upload and download, versioned snapshots with conflict
detection, history, restore, ZIP download) with storage quotas, and serves a placeholder web
page. There is no desktop client yet; don't use it for real data.

## Quick start (Docker)

```sh
cp .env.example .env        # optional: adjust settings
docker compose up -d --build
curl http://localhost:8080/health
```

Then open <http://localhost:8080>. Data lives in `./data` (mounted at `/data`). The container
starts as root only to give `./data` to the runtime user (`PUID`/`PGID`, default `10001`), then
drops privileges; set `PUID`/`PGID` in `.env` to your own `id -u`/`id -g` to own the files yourself.

## First-run setup

On its first start (no accounts yet) the server logs a one-time **setup token** at WARN level:

```sh
docker compose logs cloudit | grep setup_token
docker compose exec cloudit cloudit-server setup-token   # prints it again
```

Every instance of an HA deployment prints the same token. Use it to create the first admin
account; it stops working once setup is done. Until then the API answers `setup_required`.
To choose the token yourself (automated deployments) set `SETUP_TOKEN` (at least 16 characters).

The setup page of the web UI is not built yet (it comes with the UI milestone); for now use the
API. Browser-style requests must carry the server's origin:

```sh
curl -X POST http://localhost:8080/api/v1/setup \
  -H 'Origin: http://localhost:8080' -H 'Content-Type: application/json' \
  -d '{"token":"<setup token>","email":"you@example.com","password":"<at least 8 characters>","display_name":"You","public_url":"http://localhost:8080"}'
```

`public_url` (optional) is the address users open; it is stored as a setting (admins can
change it later) and used for sign-in provider callback URLs and links. The `PUBLIC_URL`
environment variable, if set, wins over it.

New accounts register with an invite from an admin by default (registration mode `invite`;
`open` is the alternative, changed by admins at runtime).

### Admin approval of new accounts

An account created without an invite can be made to wait for an admin's approval. The
setting `password_new_accounts` (admin settings, default `approval`) controls email/password
registrations; each sign-in provider has its own `new_accounts` (default `approval` for new
providers; `auto` = active at once).

- A **pending** account exists but can't sign in (`account_pending`, shown only after a
  correct password or provider sign-in); registering answers HTTP 202 `{"status":
  "pending"}` without signing in.
- Never waiting: accounts created with a valid **invite** (the invite is an admin's
  approval), the first admin, accounts that existed before this feature, and an OIDC account
  that is in one of the provider's **admin groups** (the provider vouches for it). Linking
  a provider to an existing account never needs approval.
- Admins see the pending accounts in `GET /api/v1/admin/users?approval_status=pending` (and
  their number in `GET /api/v1/admin/summary`), then `POST
  /api/v1/admin/users/<id>/approve` or `/reject`.
- **Rejecting** is a ban: the account can't sign in by any method, its web sessions are
  deleted, its desktop devices stop working and its provider accounts can't sign up again.
  It is kept, not deleted; approving it lifts the ban (devices work again). Admins can't
  reject themselves or the last admin.

**Locked out?** `cloudit-server admin reset-password <email>` prints a single-use password-reset
link (valid 24 hours; the full URL when `PUBLIC_URL` is set), also for accounts without a
password:

```sh
docker compose exec cloudit cloudit-server admin reset-password you@example.com
```

## Scopes, folders and sharing

A **scope** groups folders (e.g. "Games"); each **folder** is one synced unit. A scope belongs
to the user who created it and can be shared with other users by their exact email (an
approved, enabled account; there is no user directory) as:

- **viewer**: sees the scope, its folders and their history, downloads;
- **editor** (the default): also creates and renames folders and uploads new versions;
- **admin**: also moves folders to the trash, restores them and changes their retention.

Only the owner renames the scope, moves it to the trash, and adds, changes or removes
members; any member can leave. Server admins have no access to other users' scopes.

Names are unique, ignoring case, among a user's scopes and among a scope's folders.
Deleting a scope or folder moves it to the **trash** (`GET /api/v1/trash`), where it can be
restored (under another name if its old one was taken meanwhile) for `trash_retention_days`
(admin setting, default 30); after that it is deleted for good. Owners (scopes) and scope
admins (folders) can also delete trashed items for good right away
(`DELETE /api/v1/trash/scopes/{id}`, `DELETE /api/v1/trash/folders/{id}`,
`POST /api/v1/trash/empty`). A folder keeps by default the last 20 versions and at most 5 per
UTC day (admin settings `retention_max_versions`, `retention_max_per_day`; `0` =
unlimited), overridable per folder; the current version, pinned versions and unresolved
conflicts are always kept.

## Sync, versions and quotas

Each sync uploads only the content the scope doesn't have yet (files are split into
content-defined chunks, deduplicated per scope) and records a **snapshot**: an immutable
version of the folder. If two devices sync the same folder from the same version, the second
gets a conflict instead of overwriting: it keeps its state as a conflict snapshot until the
user keeps the server version or its own. Any snapshot can be restored (as a new version),
pinned, labelled or downloaded as a ZIP.

**Storage quotas** count the stored bytes of every scope a user owns (shared scopes count
against their owner, also for members' uploads; scopes in the trash still count). Admins set
a server default (`default_quota_bytes` in `PATCH /api/v1/admin/settings`, unlimited by
default) and per-user quotas (`PATCH /api/v1/admin/users/{id}` `{"quota_bytes": …}`);
`GET /api/v1/me/usage` shows a user's usage. Space is given back when old versions are
deleted (retention, trash) and the garbage collector removes the data nothing uses any more
(after `GC_GRACE`, default 24 hours).

## Real-time updates and maintenance jobs

Clients (desktop, web) open `GET /api/v1/events`, a Server-Sent Events stream announcing new
versions, conflicts, history, folder, scope and membership changes to the users who can see
them, and fetch `GET /api/v1/sync/heads` on every (re)connect. With several instances,
events cross instances through Redis/Valkey pub/sub; without it (or while it is down) each
instance only delivers its own events and clients fall back to polling `/sync/heads`.

Background jobs run on any instance, one at a time per job (a lease in the database):
**retention** (every 15 minutes), **gc** (unreferenced chunks and manifests, hourly),
**orphan_sweep** (stored objects without a database row, daily), **trash_purge** (hourly and
right after a permanent delete) and **cleanup** (expired sessions, codes and tokens, hourly).
Admins see their runs in `GET /api/v1/admin/jobs` and can start one with
`POST /api/v1/admin/jobs/{name}/run`. `JOBS_ENABLED=false` keeps an instance out of the
scheduling; `GC_GRACE` sets how old unused data must be before it is deleted.

## Sign-in providers (OIDC, GitHub, Discord)

Besides email and password (always available), users can sign in with identity providers that
admins configure at runtime through the admin API (`/api/v1/admin/providers`; the web pages
come with the UI milestone). Presets: Google, Microsoft (Entra ID), GitLab (gitlab.com or
self-hosted), authentik, Keycloak, Authelia, ZITADEL, Pocket ID, any OpenID Connect provider,
GitHub and Discord. Apple is not supported yet.

1. Set the server's public URL: `PUBLIC_URL`, or the setting stored at first-run setup /
   in the admin settings (the environment variable wins). Provider sign-in refuses to start
   without one.
2. At the provider, create an OAuth/OIDC client with the redirect (callback) URL
   `<public URL>/api/v1/auth/oauth/<provider id>/callback`, where `<provider id>` is the id you
   give the provider in cloudit (e.g. `https://sync.example.com/api/v1/auth/oauth/google/callback`).
   The admin API also shows it as `callback_url`.
3. Add the provider in cloudit with its client id and secret (and the issuer URL for
   self-hosted ones). For OIDC the server fetches the discovery document right away and
   refuses the provider if that fails.

Notes:

- **Microsoft**: set your directory's tenant id (or verified domain). The multi-tenant
  endpoints `common`, `organizations` and `consumers` are not supported: their ID tokens
  can't be matched to a single issuer.
- **Groups** (OIDC presets except Google): `groups_claim` (default `groups`), `admin_groups`
  (members become admins; recomputed at each sign-in through that provider) and
  `allowed_groups` (only members may sign in). Configure the provider to put the groups in
  the ID token or the user-info response.
- A first sign-in with an unknown provider account leads to onboarding: create a new account
  (registration mode, invites and the provider's `new_accounts` approval setting apply) or attach it to an existing one. With the
  `autolink_email` setting (off by default) an account is linked automatically only when the
  provider marks the email verified and the account's own email is verified too.
- Client secrets are stored encrypted with the server's **secret key**: `SECRET_KEY`, or
  with a single SQLite instance the file `<DATA_DIR>/secret.key` generated on first start.
  Back the key up together with the database: without it the stored client secrets are
  unreadable (re-enter them) and the server refuses to start with another key.

## Deployment

Every setting is an environment variable, documented in [`.env.example`](.env.example).
`GET /health` is the liveness probe; `GET /ready` answers 200 only when the database and blob
storage work (its JSON body reports `database`, `storage` and `redis`).

### Single instance (default)

`docker compose up -d` with no configuration runs one container with SQLite
(`/data/cloudit.db`) and filesystem blob storage (`/data/blobs`). Back up the whole `./data`
directory: it also holds `secret.key`, the generated key that encrypts the sign-in providers'
client secrets (or set `SECRET_KEY` yourself). Before applying database migrations the server copies the database to
`/data/backups/cloudit-<timestamp>-pre-migrate.db`; these copies are never deleted
automatically, prune old ones yourself.

### High availability (several instances)

Instances are stateless: run as many as you like behind a load balancer, as long as they share

- **PostgreSQL**: `DATABASE_URL=postgres://user:password@host:5432/cloudit`. Instances
  starting together coordinate migrations with an advisory lock.
- **the same `SECRET_KEY`** on every instance (required with PostgreSQL; generate it with
  `cloudit-server gen-secret-key`, e.g. `docker run --rm <image> gen-secret-key`, and keep it
  with your database backups).
- **S3-compatible storage**: `STORAGE=s3`, `S3_ENDPOINT` (unset for AWS), `S3_REGION`,
  `S3_BUCKET` (must exist), `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`,
  `S3_FORCE_PATH_STYLE` (default `true`, needed by MinIO and Garage). Garage signs with
  the region from its `s3_region` setting (usually `garage`): set `S3_REGION` to match.
- optionally **Redis or Valkey**: `REDIS_URL=redis://host:6379`, so rate limits and
  real-time events are shared across instances. If it is missing or down, every instance
  falls back to its own limits and its own events (clients poll); requests never fail
  because of Redis.
- background jobs run on every instance by default and coordinate through database leases;
  set `JOBS_ENABLED=false` on instances that should only serve requests.

Health-check the instances on `/ready`. Keep their clocks in sync (NTP): job leases, sessions
and tokens use timestamps from the instance clocks. Behind a reverse proxy set
`TRUST_PROXY_HEADERS=true` (only if clients cannot reach the instances directly) and
`PUBLIC_URL` to the address users open.

[`docker-compose.ha.yml`](docker-compose.ha.yml) is a complete example: two replicas behind
Caddy on port 8080, PostgreSQL, Garage and Valkey (`make up-ha`, `make down-ha`). Its
credentials are placeholders for local testing.

**Presigned URLs** (`S3_PRESIGN=true`, off by default) let clients upload and download blobs
directly with S3 instead of through the server. The URLs are signed for
`S3_PUBLIC_ENDPOINT` (default `S3_ENDPOINT`), which must be reachable by the clients under
exactly that host name; a reverse proxy in front of S3 must keep the `Host` header. If S3 is
on a private network, leave presigning off: the server proxies the bytes.

## Development

Requirements: Rust (stable), Node.js 24+, pnpm (version pinned in `package.json`), GNU make;
Docker for the image.

```sh
make install   # JS dependencies
make dev       # server on :8080 + web dev server on :5173 (proxies /api)
make check     # formatting, lints, tests, type freshness, web build
make test-ha   # server tests against PostgreSQL, Garage and Valkey in Docker
make help      # every target
```

Project layout and internals: [STRUCTURE.md](STRUCTURE.md). Contribution rules:
[AGENTS.md](AGENTS.md).

## Upgrade notes

The server applies database migrations when it starts. With SQLite it backs up the database
first and refuses to start if the backup fails. With PostgreSQL take a backup yourself
(`pg_dump`) before upgrading.

- **Next release (events and jobs):** no manual step is required, but read on. New tables
  (`jobs`, `job_runs`, `object_deletions`) and columns (`scopes.purge_requested_at`,
  `folders.purge_requested_at`) are created at startup. The maintenance jobs start with the
  server: the first runs apply **retention** to existing folders (by default only the last
  20 versions, at most 5 per day, are kept: raise `retention_max_versions` /
  `retention_max_per_day` in the admin settings **before upgrading** if you want to keep
  more), **delete for good** trashed items older than `trash_retention_days` (30), and free
  unused storage. New optional variables: `JOBS_ENABLED` (default `true`), `GC_GRACE`
  (default `24h`). A commit whose `parent` names an unknown (e.g. deleted) snapshot now
  answers 409 `conflict` instead of 422. Behind a reverse proxy, make sure
  `/api/v1/events` is not buffered (the server sends `X-Accel-Buffering: no`).
- **Next release (sync protocol):** nothing required. New tables (`chunks`, `manifests`,
  `snapshots`, `snapshot_chunks`) and columns (`scopes.stored_bytes`, `users.quota_bytes`)
  are created at startup; quotas are unlimited until an admin sets one. Folder responses gain
  `head` and `unresolved_conflicts`; admin user entries gain `used_bytes`, `quota_bytes`,
  `effective_quota_bytes`; error bodies may carry `head_snapshot_id`, `hashes` or `detail`.
  With `S3_PRESIGN=true` clients upload and download chunks directly with S3 (keep
  `S3_PRESIGN_TTL` short).
- **Next release (scopes and folders):** nothing required. New tables (`scopes`,
  `scope_members`, `folders`) are created at startup; `GET/PATCH /api/v1/admin/settings`
  gain `retention_max_versions`, `retention_max_per_day` and `trash_retention_days`.
- **Next release (sign-in providers):** with **PostgreSQL** set `SECRET_KEY` (same value on
  every instance, `cloudit-server gen-secret-key`) before upgrading, or the server refuses to
  start. With SQLite nothing is required: the first start writes `<DATA_DIR>/secret.key`;
  back it up with the database. New tables are created at startup. Disabling an account now
  also signs out its web sessions for good, and a new password-reset link invalidates the
  user's older unused ones. `POST /api/v1/setup` accepts `public_url`; `GET /auth/config`
  lists the enabled providers (with `kind`). **Admin approval:** password registrations
  without an invite now create pending accounts until an admin approves them (setting
  `password_new_accounts`, default `approval`): with registration mode `open`, set it to
  `auto` (`PATCH /api/v1/admin/settings`) to keep the old behaviour. Providers get
  `new_accounts`; existing providers are migrated to `approval` too (set `auto` per
  provider to keep sign-ups immediate). Existing accounts stay approved. `POST
  /auth/register` and `/auth/onboarding/create` answer 202 `{"status": "pending"}` for a
  pending account.
- **Accounts:** nothing required. New tables are created at startup; the
  first start logs a setup token (see "First-run setup"). Every `/api/v1` endpoint except
  `/health`, `/setup` and `/auth/config` answers `setup_required` until the first admin exists.
  New optional variables: `SETUP_TOKEN`, `TRUST_PROXY_HEADERS`; set `PUBLIC_URL` in production
  (CSRF origin, `Secure` cookies). Browser requests that change state must come from the
  server's origin. The HA example now uses Garage instead of MinIO: its old `minio` volume is
  no longer used (remove it with `docker volume rm cloudit-ha_minio`).
- **Storage infrastructure:** nothing required. New optional variables:
  `DATABASE_URL`, `DB_MAX_CONNECTIONS`, `STORAGE`, `STORAGE_FS_DIR`, `S3_*`, `REDIS_URL`
  (see `.env.example`). The defaults keep everything under `DATA_DIR`. `GET /ready` now
  checks the database and storage and returns a different JSON body (`status`, `version`,
  `components`).
