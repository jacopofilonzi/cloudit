// Sets the project version everywhere it lives:
// - `[workspace.package] version` in Cargo.toml (all crates inherit it);
// - `version` in package.json (root, apps/*, packages/*);
// - `version` in apps/desktop/src-tauri/tauri.conf.json (the installers and the updater).
//
// Usage: node scripts/set-version.mjs x.y.z   (or `make version V=x.y.z`)
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const version = process.argv[2] ?? '';

if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
  console.error(`usage: node scripts/set-version.mjs x.y.z (got ${JSON.stringify(version)})`);
  process.exit(1);
}

const updated = [];

// Cargo.toml: only the `version` line inside `[workspace.package]`.
const cargoPath = join(root, 'Cargo.toml');
const cargo = readFileSync(cargoPath, 'utf8');
const section = /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]*(")/;
if (!section.test(cargo)) {
  console.error('set-version: no `version` in [workspace.package] of Cargo.toml');
  process.exit(1);
}
writeFileSync(cargoPath, cargo.replace(section, `$1${version}$2`));
updated.push(cargoPath);

// JSON manifests: rewrite only the top-level "version" field, keeping formatting.
function setJsonVersion(path) {
  if (!existsSync(path)) return;
  const text = readFileSync(path, 'utf8');
  const parsed = JSON.parse(text);
  if (typeof parsed.version !== 'string') return;
  const next = text.replace(/^(\s*"version"\s*:\s*")[^"]*(")/m, `$1${version}$2`);
  if (JSON.parse(next).version !== version) {
    console.error(`set-version: could not update ${path}`);
    process.exit(1);
  }
  writeFileSync(path, next);
  updated.push(path);
}

setJsonVersion(join(root, 'package.json'));
for (const group of ['apps', 'packages']) {
  const dir = join(root, group);
  if (!existsSync(dir)) continue;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) setJsonVersion(join(dir, entry.name, 'package.json'));
  }
}
setJsonVersion(join(root, 'apps', 'desktop', 'src-tauri', 'tauri.conf.json'));

for (const path of updated) console.info(`set-version: ${relative(root, path)} -> ${version}`);

// Cargo.lock records the workspace crates' versions; refresh only those
// (CI builds with --locked).
const lock = spawnSync('cargo', ['update', '--workspace', '--offline'], {
  cwd: root,
  stdio: 'inherit'
});
if (lock.error || lock.status !== 0) {
  console.error('set-version: `cargo update --workspace` failed; run it before committing');
  process.exit(1);
}
console.info('set-version: Cargo.lock updated');
