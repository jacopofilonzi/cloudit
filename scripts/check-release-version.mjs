// Fails unless a release tag matches the versions of its side (files listed in
// scripts/versions.mjs): `app-vX.Y.Z` the desktop app's (so a release can't ship
// installers whose updater version disagrees with its tag), `server-vX.Y.Z` the
// server's (so the Docker image tags match the server it contains); and that the side's
// changelog (scripts/changelog.mjs) has entries for the version, so a release never ships
// without its notes.
//
// Usage: node scripts/check-release-version.mjs app-v1.2.3   (the release workflows pass the tag)
import { readFileSync } from 'node:fs';
import { relative } from 'node:path';

import { changelogFile, section } from './changelog.mjs';
import { CARGO_VERSION, root, versionFiles } from './versions.mjs';

const tag = process.argv[2] ?? '';
const match = /^(app|server)-v(\d+\.\d+\.\d+)$/.exec(tag);
if (!match) {
  console.error(
    `check-release-version: ${JSON.stringify(tag)} is not an app-vX.Y.Z or server-vX.Y.Z tag`
  );
  process.exit(1);
}
const [, side, version] = match;
const files = versionFiles(side);

const found = [
  ...files.cargo.map((path) => [path, CARGO_VERSION.exec(readFileSync(path, 'utf8'))?.[2]]),
  ...files.json.map((path) => [path, JSON.parse(readFileSync(path, 'utf8')).version])
];
let ok = true;
for (const [path, value] of found) {
  if (value !== version) {
    console.error(
      `check-release-version: ${relative(root, path)} has ${value ?? 'no version'}, the tag ${tag}`
    );
    ok = false;
  }
}
const changelog = changelogFile(side);
if (!section(readFileSync(changelog, 'utf8'), version)) {
  console.error(
    `check-release-version: ${relative(root, changelog)} has no entries for ${version} ("## [${version}] - …")`
  );
  ok = false;
}
if (!ok) process.exit(1);
console.info(
  `check-release-version: ${tag} matches the ${side} versions (${found.length} files) and has its changelog section`
);
