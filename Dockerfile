# syntax=docker/dockerfile:1

# ---- web: build the SvelteKit SPA ------------------------------------------
FROM node:24-trixie-slim AS web
WORKDIR /src
# corepack reads `packageManager` from package.json and fetches that pnpm.
RUN npm install -g corepack@latest && corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/
COPY packages/api-types/package.json packages/api-types/
COPY packages/ui/package.json packages/ui/
RUN --mount=type=cache,id=pnpm-store,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
COPY apps/web apps/web
COPY packages packages
RUN pnpm --filter @cloudit/web build

# ---- server: build the release binary --------------------------------------
FROM rust:1.99-slim-trixie AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
# Cache mounts keep the registry and compiled dependencies between builds.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p cloudit-server \
    && cp target/release/cloudit-server /usr/local/bin/cloudit-server

# ---- runtime ---------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin cloudit \
    && mkdir -p /data \
    && chown cloudit:cloudit /data
COPY --from=server /usr/local/bin/cloudit-server /usr/local/bin/cloudit-server
COPY --from=web /src/apps/web/build /app/web

ENV BIND_ADDR=0.0.0.0 \
    PORT=8080 \
    DATA_DIR=/data \
    WEB_DIR=/app/web \
    LOG_FORMAT=json
USER cloudit
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["cloudit-server", "healthcheck"]
ENTRYPOINT ["cloudit-server"]
