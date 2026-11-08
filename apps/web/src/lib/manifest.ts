// File lists of snapshots: sorting, searching and the difference between two
// manifests (the "compare" of a conflict). Pure, so it is unit-tested.
import type { Manifest, ManifestEntry } from '@cloudit/api-types';

export interface ManifestChange {
  path: string;
  before: ManifestEntry;
  after: ManifestEntry;
}

export interface ManifestDiff {
  /** In `other` only. */
  added: ManifestEntry[];
  /** In `base` only. */
  removed: ManifestEntry[];
  /** In both, with another kind, size, content or executable bit. */
  modified: ManifestChange[];
  /** Same path and content (the modification time may differ: nothing to transfer). */
  unchanged: number;
}

function sameContent(a: ManifestEntry, b: ManifestEntry): boolean {
  if (a.kind !== b.kind || a.size !== b.size || a.executable !== b.executable) return false;
  if (a.chunks.length !== b.chunks.length) return false;
  return a.chunks.every((chunk, i) => chunk.hash === b.chunks[i]?.hash);
}

/** What `other` has that `base` hasn't, and vice versa; each list in path order. */
export function diffManifests(base: Manifest, other: Manifest): ManifestDiff {
  const before = new Map(base.entries.map((entry) => [entry.path, entry]));
  const diff: ManifestDiff = { added: [], removed: [], modified: [], unchanged: 0 };
  const seen = new Set<string>();
  for (const after of other.entries) {
    seen.add(after.path);
    const old = before.get(after.path);
    if (!old) diff.added.push(after);
    else if (sameContent(old, after)) diff.unchanged += 1;
    else diff.modified.push({ path: after.path, before: old, after });
  }
  for (const entry of base.entries) {
    if (!seen.has(entry.path)) diff.removed.push(entry);
  }
  const byPath = (a: { path: string }, b: { path: string }) =>
    a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
  diff.added.sort(byPath);
  diff.removed.sort(byPath);
  diff.modified.sort(byPath);
  return diff;
}

/** `["saves/slot1.sav"]` → directory `saves/` and file name `slot1.sav`. */
export function splitPath(path: string): { dir: string; name: string } {
  const index = path.lastIndexOf('/');
  return index === -1
    ? { dir: '', name: path }
    : { dir: path.slice(0, index + 1), name: path.slice(index + 1) };
}

/** Entries whose path contains `query` (case-insensitive); all of them when empty. */
export function filterEntries(entries: readonly ManifestEntry[], query: string): ManifestEntry[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [...entries];
  return entries.filter((entry) => entry.path.toLocaleLowerCase().includes(needle));
}

/** The modification time as a `Date`. */
export function mtimeOf(entry: ManifestEntry): Date {
  return new Date(entry.mtime.secs * 1000 + Math.floor(entry.mtime.nanos / 1_000_000));
}
