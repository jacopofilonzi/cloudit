import { describe, expect, it } from 'vitest';
import type { FolderStatus, LinkedFolder } from '@cloudit/api-types';
import {
  DISPLAY_STATES,
  changeCounts,
  displayState,
  fileRows,
  needsAttention,
  primaryAction
} from './status';

const link = (over: Partial<LinkedFolder> = {}): LinkedFolder => ({
  id: 1,
  local_path: '/home/u/x',
  scope_id: 's',
  folder_id: 'f',
  scope_name: 'scope',
  folder_name: 'folder',
  emoji: null,
  ignore: [],
  paused: false,
  base_snapshot_id: null,
  pending_conflict_id: null,
  last_state: null,
  last_error: null,
  last_sync_at: null,
  created_at: '2026-01-01T00:00:00Z',
  ...over
});

const status = (over: Partial<FolderStatus>): FolderStatus => ({
  folder: 1,
  state: 'in_sync',
  paused: false,
  local_changes: [],
  remote_changes: [],
  conflicting: [],
  problems: [],
  base_snapshot_id: null,
  head_snapshot_id: null,
  unresolved_conflicts: 0,
  pending_conflict_id: null,
  ...over
});

describe('displayState', () => {
  it('uses the fresh status, then the last known state', () => {
    expect(displayState(link({ last_state: 'in_sync' }), status({ state: 'local_changes' }))).toBe(
      'local_changes'
    );
    expect(displayState(link({ last_state: 'remote_changes' }))).toBe('remote_changes');
    expect(displayState(link())).toBe('unknown');
  });

  it('shows paused only when nothing needs the user', () => {
    expect(displayState(link({ paused: true }), status({ state: 'in_sync' }))).toBe('paused');
    expect(displayState(link({ paused: true }), status({ state: 'conflict' }))).toBe('conflict');
  });
});

describe('actions', () => {
  it('offers one main action per state and flags what needs attention', () => {
    expect(primaryAction('local_changes')).toBe('upload');
    expect(primaryAction('remote_changes')).toBe('download');
    expect(primaryAction('conflict')).toBe('resolve');
    expect(primaryAction('missing_local_path')).toBe('relink');
    for (const state of DISPLAY_STATES) expect(primaryAction(state)).toBeTruthy();
    expect(needsAttention('in_sync')).toBe(false);
    expect(needsAttention('paused')).toBe(false);
    expect(needsAttention('conflict')).toBe(true);
  });
});

describe('changes', () => {
  it('counts changes and merges both sides into file rows', () => {
    const s = status({
      local_changes: [
        { path: 'a', change: 'added' },
        { path: 'b', change: 'modified' }
      ],
      remote_changes: [{ path: 'c', change: 'removed' }],
      conflicting: ['b']
    });
    expect(changeCounts(s.local_changes)).toEqual({ added: 1, modified: 1, removed: 0 });
    const rows = fileRows(s);
    expect(rows.map((r) => r.path)).toEqual(['a', 'b', 'c']);
    expect(rows[1]).toMatchObject({ local: 'modified', remote: null, both: true });
  });
});
