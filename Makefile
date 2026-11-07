# Dev commands. Recipes must run in both sh and cmd.exe (GNU make on Windows):
# only `cd dir && cmd`, no inline `VAR=x cmd`, no rm/cp/mkdir -p (use node scripts).
# Targets documented with `## ` show up in `make help`.

.DEFAULT_GOAL := help
.PHONY: help install dev dev-server dev-web types fmt fmt-check lint test check check-types build docker up down up-ha down-ha test-ha clean version

IMAGE ?= cloudit:dev

help: ## List the available targets
	@node scripts/help.mjs Makefile

install: ## Install JS dependencies (pnpm workspace)
	pnpm install

dev: ## Run the server (cargo run) and the web dev server together
	node scripts/dev.mjs

dev-server: ## Run only the Rust server (http://localhost:8080)
	cargo run -p cloudit-server

dev-web: ## Run only the Vite dev server (proxies /api to :8080)
	pnpm --filter @cloudit/web dev

types: ## Regenerate TS API types from Rust (ts-rs) into packages/api-types
	node scripts/types.mjs

check-types: ## Fail if the generated TS API types are stale
	node scripts/types.mjs --check

fmt: ## Format Rust and JS/TS/Svelte code
	cargo fmt --all
	pnpm exec prettier --write .

fmt-check: ## Check formatting without writing
	cargo fmt --all --check
	pnpm exec prettier --check .

lint: ## Clippy (deny warnings), ESLint, svelte-check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	pnpm exec eslint --max-warnings 0 .
	pnpm -r --if-present run check

test: ## Rust and JS unit tests
	cargo test --workspace --locked
	pnpm -r --if-present run test

check: fmt-check lint test check-types ## Everything CI runs: formatting, lints, tests, type freshness, web build
	pnpm --filter @cloudit/web build

build: ## Build the web UI and the release server binary
	pnpm --filter @cloudit/web build
	cargo build --release --locked -p cloudit-server

docker: ## Build the Docker image ($(IMAGE))
	docker build -t $(IMAGE) .

up: ## Start the single-instance stack (docker compose)
	docker compose up -d --build

down: ## Stop the docker compose stack
	docker compose down

up-ha: ## Start the HA example stack (2 replicas, Caddy on :8080, PostgreSQL, MinIO, Valkey)
	docker compose -f docker-compose.ha.yml up -d --build

down-ha: ## Stop the HA example stack (volumes kept; add -v by hand to wipe them)
	docker compose -f docker-compose.ha.yml down

test-ha: ## Server tests against PostgreSQL, MinIO and Valkey in Docker (cleaned up after)
	node scripts/test-ha.mjs

clean: ## Remove build outputs (target, web build, .svelte-kit)
	cargo clean
	node scripts/clean.mjs

version: ## Set the version everywhere: make version V=x.y.z
	node scripts/set-version.mjs $(V)
