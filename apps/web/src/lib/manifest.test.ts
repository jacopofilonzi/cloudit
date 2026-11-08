import { describe, expect, it } from 'vitest';
import type { Manifest, ManifestEntry } from '@cloudit/api-types';
import { diffManifests, filterEntries, splitPath } from './manifest';

const HASH_A = 'a'.repeat(64);
const HASH_B = 'b'.repeat(64);

function file(path: string, hash: string, size = 10, secs = 1): ManifestEntry {
  return {
    path,
    kind: 'file',
    size,
    mtime: { secs, nanos: 0 },
    executable: false,
    chunks: [{ hash, size }]
  };
}

function manifest(...entries: ManifestEntry[]): Manifest {
  return { version: 1, entries };
}

describe('diffManifests', () => {
  it('finds added, removed, modified and unchanged entries', () => {
    const base = manifest(file('a.txt', HASH_A), file('b.txt', HASH_A), file('c.txt', HASH_A));
    const other = manifest(
      file('a.txt', HASH_A, 10, 99), // only the mtime differs
      file('b.txt', HASH_B),
      file('d.txt', HASH_B)
    );
    const diff = diffManifests(base, other);
    expect(diff.unchanged).toBe(1);
    expect(diff.modified.map((m) => m.path)).toEqual(['b.txt']);
    expect(diff.added.map((e) => e.path)).toEqual(['d.txt']);
    expect(diff.removed.map((e) => e.path)).toEqual(['c.txt']);
  });

  it('treats a size or executable change as modified', () => {
    const base = manifest(file('a', HASH_A, 10));
    const bigger = manifest(file('a', HASH_A, 11));
    expect(diffManifests(base, bigger).modified).toHaveLength(1);
    const exec = manifest({ ...file('a', HASH_A, 10), executable: true });
    expect(diffManifests(base, exec).modified).toHaveLength(1);
  });
});

describe('paths', () => {
  it('splits a path in directory and name', () => {
    expect(splitPath('saves/world/level.dat')).toEqual({ dir: 'saves/world/', name: 'level.dat' });
    expect(splitPath('readme.txt')).toEqual({ dir: '', name: 'readme.txt' });
  });

  it('filters by a case-insensitive part of the path', () => {
    const entries = [file('Saves/Slot1.sav', HASH_A), file('config.ini', HASH_A)];
    expect(filterEntries(entries, 'SLOT').map((e) => e.path)).toEqual(['Saves/Slot1.sav']);
    expect(filterEntries(entries, '  ')).toHaveLength(2);
  });
});
