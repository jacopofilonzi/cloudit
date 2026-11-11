// The two independently versioned sides of the project and the files that hold their
// version (used by set-version.mjs and check-release-version.mjs).
//
// - server (tag `server-vX.Y.Z`): crates/core, crates/server, apps/web, packages/*,
//   the root package.json;
// - app (tag `app-vX.Y.Z`): crates/client, apps/desktop (its package.json, the
//   src-tauri crate and tauri.conf.json, which the installers and the updater use).
import { existsSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = join(dirname(fileURLToPath(import.meta.url)), '..');

function packages() {
  const dir = join(root, 'packages');
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(dir, entry.name, 'package.json')))
    .map((entry) => join(dir, entry.name, 'package.json'));
}

/** `{cargo: Cargo.toml paths, json: JSON manifests with a top-level "version"}`. */
export function versionFiles(side) {
  switch (side) {
    case 'server':
      return {
        cargo: [
          join(root, 'crates', 'core', 'Cargo.toml'),
          join(root, 'crates', 'server', 'Cargo.toml')
        ],
        json: [join(root, 'package.json'), join(root, 'apps', 'web', 'package.json'), ...packages()]
      };
    case 'app':
      return {
        cargo: [
          join(root, 'crates', 'client', 'Cargo.toml'),
          join(root, 'apps', 'desktop', 'src-tauri', 'Cargo.toml')
        ],
        json: [
          join(root, 'apps', 'desktop', 'package.json'),
          join(root, 'apps', 'desktop', 'src-tauri', 'tauri.conf.json')
        ]
      };
    default:
      return null;
  }
}

/** The `version` of the `[package]` section of a Cargo.toml's text (or undefined). */
export const CARGO_VERSION = /(\[package\][^[]*?\nversion\s*=\s*")([^"]*)(")/;
