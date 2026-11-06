# cloudit

> Provisional name.

cloudit is a self-hosted client-server folder sync service: point the desktop client at a
folder (game saves, configs, documents) and it keeps versioned snapshots of it on your own
server, synced across your devices. Folders are grouped in per-user scopes that can be shared
with other users.

**Status: early development.** Only the project scaffolding exists so far: the server answers
health checks and serves a placeholder web page. Nothing syncs yet; don't use it for real data.

## Quick start (Docker)

```sh
cp .env.example .env        # optional: adjust settings
docker compose up -d --build
curl http://localhost:8080/health
```

Then open <http://localhost:8080>. Data lives in `./data` (mounted at `/data`). The container
starts as root only to give `./data` to the runtime user (`PUID`/`PGID`, default `10001`), then
drops privileges; set `PUID`/`PGID` in `.env` to your own `id -u`/`id -g` to own the files yourself.

## Development

Requirements: Rust (stable), Node.js 24+, pnpm (version pinned in `package.json`), GNU make;
Docker for the image.

```sh
make install   # JS dependencies
make dev       # server on :8080 + web dev server on :5173 (proxies /api)
make check     # formatting, lints, tests, type freshness, web build
make help      # every target
```

Project layout and internals: [STRUCTURE.md](STRUCTURE.md). Contribution rules:
[AGENTS.md](AGENTS.md).

## Upgrade notes

None yet.
