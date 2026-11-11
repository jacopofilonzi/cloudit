# Changelog: cloudit server

The server, its web UI and the Docker image (`ghcr.io/jacopofilonzi/cloudit`). The desktop app
has its own changelog: [CHANGELOG-desktop.md](CHANGELOG-desktop.md). The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are `x.y.z` (a server
version `X.Y.Z` is the git tag `server-vX.Y.Z` and the image tags `X.Y.Z`, `X.Y`, `latest`).

## [Unreleased]

## [0.2.0] - 2026-11-11

### Added

- `/health` announces the server's API level and the oldest desktop app version it accepts, so
  apps can tell when one of the two must be updated.
- Admins can give an account unlimited storage even when the server has a default quota
  ("Unlimited" in the quota dialog). The account's own quota is kept: turning "Unlimited"
  off again brings it back (or the server default if it had none).
- Chunk packs: desktop apps that support them upload and download many small chunks in one
  request, which makes syncing folders with many small files much faster.
- The update check also runs when an admin opens the web app (at most every 10 minutes for
  the whole server), and admins can start one with "Check now" in the administration.

### Changed

- The server has its own version and releases, separate from the desktop app. A server release
  publishes only the Docker image; its release notes are this file (the repository's GitHub
  Releases are the desktop app's).
- The update check looks for the `server-vX.Y.Z` tags instead of GitHub Releases, and the
  update notice links to this changelog.

## [0.1.3] - 2026-11-10

### Changed

- No server changes: released together with the desktop app.

## [0.1.2] - 2026-11-10

### Added

- Multi-architecture Docker image (amd64 and arm64).
- A production compose file that runs the published image.

### Changed

- The first-run setup page no longer shows the sign-in providers note.

### Fixed

- `.env.example` no longer has uncommented lines that override the Docker image's paths.

## [0.1.1] - 2026-11-10

First release.

### Added

- Self-hosted folder sync server: scopes (collections of folders) shared with roles, folders
  synced as versioned snapshots, history, restore, pinning and labels, conflicts kept as
  versions, ZIP downloads, storage usage and quotas, trash with retention.
- Accounts: first-run setup, password accounts, invites, approval of new accounts, password
  reset links, desktop device sign-in, sign-in through OIDC providers, GitHub and Discord.
- Web UI: scopes and folders, history and file comparison, sharing, devices, trash, profile,
  light and dark theme, English and Italian, and the administration (users, invites,
  sign-in providers, settings, maintenance).
- SQLite or PostgreSQL, local disk or S3-compatible storage, optional Redis or Valkey,
  several instances behind a load balancer.
- Real-time updates, background maintenance jobs and a daily check for a newer server version.
