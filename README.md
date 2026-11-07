# cloudit

> Provisional name.

cloudit is a self-hosted client-server folder sync service: point the desktop client at a
folder (game saves, configs, documents) and it keeps versioned snapshots of it on your own
server, synced across your devices. Folders are grouped in per-user scopes that can be shared
with other users.

**Status: early development.** The server has its storage infrastructure (database, blob
storage, optional Redis) and serves a placeholder web page. Nothing syncs yet; don't use it for
real data.

## Quick start (Docker)

```sh
cp .env.example .env        # optional: adjust settings
docker compose up -d --build
curl http://localhost:8080/health
```

Then open <http://localhost:8080>. Data lives in `./data` (mounted at `/data`). The container
starts as root only to give `./data` to the runtime user (`PUID`/`PGID`, default `10001`), then
drops privileges; set `PUID`/`PGID` in `.env` to your own `id -u`/`id -g` to own the files yourself.

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
  `S3_FORCE_PATH_STYLE` (default `true`, needed by MinIO and Garage).
- optionally **Redis or Valkey**: `REDIS_URL=redis://host:6379`, so rate limits are shared
  across instances. If it is missing or down, every instance falls back to its own limits;
  requests never fail because of Redis.

Health-check the instances on `/ready`. Keep their clocks in sync (NTP): job leases use
timestamps from the instance clocks.

[`docker-compose.ha.yml`](docker-compose.ha.yml) is a complete example: two replicas behind
Caddy on port 8080, PostgreSQL, MinIO and Valkey (`make up-ha`, `make down-ha`). Its
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
make test-ha   # server tests against PostgreSQL, MinIO and Valkey in Docker
make help      # every target
```

Project layout and internals: [STRUCTURE.md](STRUCTURE.md). Contribution rules:
[AGENTS.md](AGENTS.md).

## Upgrade notes

The server applies database migrations when it starts. With SQLite it backs up the database
first and refuses to start if the backup fails. With PostgreSQL take a backup yourself
(`pg_dump`) before upgrading.

- **Next release (storage infrastructure):** nothing required. New optional variables:
  `DATABASE_URL`, `DB_MAX_CONNECTIONS`, `STORAGE`, `STORAGE_FS_DIR`, `S3_*`, `REDIS_URL`
  (see `.env.example`). The defaults keep everything under `DATA_DIR`. `GET /ready` now
  checks the database and storage and returns a different JSON body (`status`, `version`,
  `components`).
