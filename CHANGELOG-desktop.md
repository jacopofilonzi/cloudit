# Changelog: cloudit desktop app

The desktop app (Windows and Linux) and its sync engine. The server has its own changelog:
[CHANGELOG-server.md](CHANGELOG-server.md). The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are `x.y.z` (an app version
`X.Y.Z` is the git tag `app-vX.Y.Z` and the GitHub Release with its installers).

## [Unreleased]

### Added

- The app checks that it and the server are compatible (at sign-in, before syncing and on
  reconnects); when one of them is too old it stops syncing and says which one to update.
  When the app is the one too old, the window shows only an "update required" screen with
  the update (or a sign-out, to use another server) until the app is updated.
- With servers that support it, many small files are uploaded and downloaded in packs, which
  makes syncing folders with many small files much faster.

### Changed

- The app has its own version and releases, separate from the server.

### Removed

- The note saying the server is out of date (the compatibility check replaces it).

## [0.1.3] - 2026-11-10

### Fixed

- A folder with an associated program no longer syncs at all while the program runs; before,
  it kept uploading and only held back downloads.

## [0.1.2] - 2026-11-10

### Changed

- The sign-in screen no longer shows the browser hint under the sign-in button.

## [0.1.1] - 2026-11-10

First release.

### Added

- Sign-in through the browser, the token kept in the system keychain.
- Linking local folders to server folders, manual sync with progress, conflicts decided by
  you (keep this computer's version, keep the server's, decide later); a file is never
  overwritten or deleted unless its content is already stored on the server.
- Automatic sync per folder, after a quiet period or when an associated program exits.
- Tray icon with the folders that need attention, pause for one hour, start at login,
  notifications, the activity log, English and Italian.
- Installers for Windows (NSIS) and Linux (AppImage, deb) and an in-app updater.
