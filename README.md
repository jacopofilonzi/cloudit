# cloudit

> Provisional name.

cloudit is a self-hosted client-server folder sync service: point the desktop client at a
folder (game saves, configs, documents) and it keeps versioned snapshots of it on your own
server, synced across your devices. Folders are grouped in per-user scopes that can be shared
with other users.

**Status: early development.** The server has its storage infrastructure (database, blob
storage, optional Redis), the account API (setup, password accounts, invites, desktop device
login, admin) and serves a placeholder web page. Nothing syncs yet; don't use it for real data.

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
  -d '{"token":"<setup token>","email":"you@example.com","password":"<at least 8 characters>","display_name":"You"}'
```

New accounts register with an invite from an admin by default (registration mode `invite`;
`open` and `closed` are the alternatives, changed by admins at runtime).

**Locked out?** `cloudit-server admin reset-password <email>` prints a single-use password-reset
link (valid 24 hours; the full URL when `PUBLIC_URL` is set), also for accounts without a
password:

```sh
docker compose exec cloudit cloudit-server admin reset-password you@example.com
```

## Deployment

Every setting is an environment variable, documented in [`.env.example`](.env.example).
`GET /health` is the liveness probe; `GET /ready` answers 200 only when the database and blob
storage work (its JSON body reports `database`, `storage` and `redis`).

### Single instance (default)

`docker compose up -d` with no configuration runs one container with SQLite
(`/data/cloudit.db`) and filesystem blob storage (`/data/blobs`). Back up the whole `./data`
directory. Before applying database migrations the server copies the database to
`/data/backups/cloudit-<timestamp>-pre-migrate.db`; these copies are never deleted
automatically, prune old ones yourself.

### High availability (several instances)

Instances are stateless: run as many as you like behind a load balancer, as long as they share

- **PostgreSQL**: `DATABASE_URL=postgres://user:password@host:5432/cloudit`. Instances
  starting together coordinate migrations with an advisory lock.
- **S3-compatible storage**: `STORAGE=s3`, `S3_ENDPOINT` (unset for AWS), `S3_REGION`,
  `S3_BUCKET` (must exist), `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`,
  `S3_FORCE_PATH_STYLE` (default `true`, needed by MinIO and Garage). Garage signs with
  the region from its `s3_region` setting (usually `garage`): set `S3_REGION` to match.
- optionally **Redis or Valkey**: `REDIS_URL=redis://host:6379`, so rate limits are shared
  across instances. If it is missing or down, every instance falls back to its own limits;
  requests never fail because of Redis.

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

- **Next release (accounts):** nothing required. New tables are created at startup; the
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
