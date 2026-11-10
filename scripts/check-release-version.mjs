// Fails unless a release tag (`vX.Y.Z`) matches the version in Cargo.toml
// (`[workspace.package]`) and in apps/desktop/src-tauri/tauri.conf.json, so a release
// can't ship installers whose updater version disagrees with its tag.
//
// Usage: node scripts/check-release-version.mjs v1.2.3   (release.yml passes the tag)
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const tag = process.argv[2] ?? '';
const match = /^v(\d+\.\d+\.\d+)$/.exec(tag);
if (!match) {
  console.error(`check-release-version: ${JSON.stringify(tag)} is not a vX.Y.Z tag`);
  process.exit(1);
}
const version = match[1];

const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8');
const section = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(cargo);
const tauri = JSON.parse(
  readFileSync(join(root, 'apps', 'desktop', 'src-tauri', 'tauri.conf.json'), 'utf8')
);
const found = { 'Cargo.toml': section?.[1], 'tauri.conf.json': tauri.version };
let ok = true;
for (const [file, value] of Object.entries(found)) {
  if (value !== version) {
    console.error(`check-release-version: ${file} has ${value ?? 'no version'}, the tag ${tag}`);
    ok = false;
  }
}
if (!ok) process.exit(1);
console.info(`check-release-version: ${tag} matches Cargo.toml and tauri.conf.json`);
