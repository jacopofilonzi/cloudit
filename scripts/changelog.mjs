// The two changelogs (Keep a Changelog style): CHANGELOG-server.md (server, web UI, Docker
// image) and CHANGELOG-desktop.md (desktop app, client engine). Used by
// check-release-version.mjs (a release tag needs its section), set-version.mjs (moves
// `[Unreleased]` to the new version) and release-app.yml (the GitHub Release body).
//
// CLI: node scripts/changelog.mjs section <file> <x.y.z>   prints that version's section
//      (its body, without the heading); fails if it is missing or empty.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { root } from './versions.mjs';

/** The changelog of a side (`server` | `app`). */
export function changelogFile(side) {
  return join(root, side === 'app' ? 'CHANGELOG-desktop.md' : 'CHANGELOG-server.md');
}

const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The heading line `## [<name>]…` and the body up to the next `## ` heading, or null. */
function find(text, name) {
  const heading = new RegExp(`^## \\[${escape(name)}\\].*$`, 'm');
  const match = heading.exec(text);
  if (!match) return null;
  const start = match.index + match[0].length;
  const next = /^## /m.exec(text.slice(start));
  const end = next ? start + next.index : text.length;
  return { index: match.index, start, end, body: text.slice(start, end).trim() };
}

/** The body of version `version`'s section (trimmed; '' when empty), or null if missing. */
export function section(text, version) {
  return find(text, version)?.body ?? null;
}

/**
 * Moves the `[Unreleased]` entries under a new `## [version] - date` heading, leaving an
 * empty `[Unreleased]` above it. Returns the new text, or the same text when the version
 * already has a section. Throws when there is no `[Unreleased]` heading.
 */
export function release(text, version, date) {
  if (find(text, version)) return text;
  const unreleased = find(text, 'Unreleased');
  if (!unreleased) throw new Error('no "## [Unreleased]" section');
  const body = unreleased.body ? `${unreleased.body}\n\n` : '';
  return (
    `${text.slice(0, unreleased.start)}\n\n## [${version}] - ${date}\n\n${body}` +
    text.slice(unreleased.end).replace(/^\n+/, '')
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const [command, file, version] = process.argv.slice(2);
  if (command !== 'section' || !file || !version) {
    console.error('usage: node scripts/changelog.mjs section <file> <x.y.z>');
    process.exit(1);
  }
  const body = section(readFileSync(file, 'utf8'), version);
  if (!body) {
    console.error(`changelog: ${file} has no entries for ${version} ("## [${version}] - …")`);
    process.exit(1);
  }
  process.stdout.write(`${body}\n`);
}
