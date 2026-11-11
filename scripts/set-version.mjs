// Sets the version of one side of the project: the server and the desktop app are
// versioned and released separately (files of each side: scripts/versions.mjs).
//
// Usage: node scripts/set-version.mjs server|app x.y.z
//        (or `make version-server V=x.y.z` / `make version-app V=x.y.z`)
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { relative } from 'node:path';

import { CARGO_VERSION, root, versionFiles } from './versions.mjs';

const side = process.argv[2] ?? '';
const version = process.argv[3] ?? '';
const files = versionFiles(side);

if (!files || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
  console.error(
    `usage: node scripts/set-version.mjs server|app x.y.z (got ${JSON.stringify(side)} ${JSON.stringify(version)})`
  );
  process.exit(1);
}

const updated = [];

// Cargo.toml: only the `version` line of the `[package]` section.
for (const path of files.cargo) {
  const text = readFileSync(path, 'utf8');
  if (!CARGO_VERSION.test(text)) {
    console.error(`set-version: no \`version\` in [package] of ${relative(root, path)}`);
    process.exit(1);
  }
  writeFileSync(path, text.replace(CARGO_VERSION, `$1${version}$3`));
  updated.push(path);
}

// JSON manifests: rewrite only the top-level "version" field, keeping formatting.
for (const path of files.json) {
  const text = readFileSync(path, 'utf8');
  const next = text.replace(/^(\s*"version"\s*:\s*")[^"]*(")/m, `$1${version}$2`);
  if (JSON.parse(next).version !== version) {
    console.error(`set-version: could not update ${relative(root, path)}`);
    process.exit(1);
  }
  writeFileSync(path, next);
  updated.push(path);
}

for (const path of updated) console.info(`set-version: ${relative(root, path)} -> ${version}`);

// Cargo.lock records the workspace crates' versions; refresh only those (the other
// side's are unchanged, so they stay as they are; CI builds with --locked).
const lock = spawnSync('cargo', ['update', '--workspace', '--offline'], {
  cwd: root,
  stdio: 'inherit'
});
if (lock.error || lock.status !== 0) {
  console.error('set-version: `cargo update --workspace` failed; run it before committing');
  process.exit(1);
}
console.info('set-version: Cargo.lock updated');
