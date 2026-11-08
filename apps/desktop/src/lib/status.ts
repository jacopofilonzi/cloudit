// How a linked folder is presented: the state shown (the engine's `SyncState`, or
// "paused" / "not checked yet"), its tag, the main action offered and the counts of a
// change list. Pure functions, no UI.
import type { FolderStatus, LinkedFolder, PathChange, SyncState } from '@cloudit/api-types';
import type { TagVariant } from '@cloudit/ui';
import type { MessageKey } from '@cloudit/ui/i18n';

export type DisplayState = SyncState | 'paused' | 'unknown';

/** Every state, in the order of the engine's precedence (for exhaustive maps and tests). */
export const DISPLAY_STATES = [
  'missing_local_path',
  'remote_missing',
  'error',
  'conflict',
  'local_changes',
  'remote_changes',
  'server_conflict',
  'in_sync',
  'paused',
  'unknown'
] as const satisfies readonly DisplayState[];

export const STATE_LABEL: Record<DisplayState, MessageKey> = {
  missing_local_path: 'desktop.state.missing_local_path',
  remote_missing: 'desktop.state.remote_missing',
  error: 'desktop.state.error',
  conflict: 'desktop.state.conflict',
  local_changes: 'desktop.state.local_changes',
  remote_changes: 'desktop.state.remote_changes',
  server_conflict: 'desktop.state.server_conflict',
  in_sync: 'desktop.state.in_sync',
  paused: 'desktop.state.paused',
  unknown: 'desktop.state.unknown'
};

export const STATE_HINT: Record<DisplayState, MessageKey> = {
  missing_local_path: 'desktop.state_hint.missing_local_path',
  remote_missing: 'desktop.state_hint.remote_missing',
  error: 'desktop.state_hint.error',
  conflict: 'desktop.state_hint.conflict',
  local_changes: 'desktop.state_hint.local_changes',
  remote_changes: 'desktop.state_hint.remote_changes',
  server_conflict: 'desktop.state_hint.server_conflict',
  in_sync: 'desktop.state_hint.in_sync',
  paused: 'desktop.state_hint.paused',
  unknown: 'desktop.state_hint.unknown'
};

export const STATE_TAG: Record<DisplayState, TagVariant> = {
  missing_local_path: 'danger',
  remote_missing: 'danger',
  error: 'danger',
  conflict: 'warning',
  local_changes: 'primary',
  remote_changes: 'primary',
  server_conflict: 'warning',
  in_sync: 'success',
  paused: 'neutral',
  unknown: 'neutral'
};

/** The state to show: the fresh status, else the last known one; paused wins when quiet. */
export function displayState(link: LinkedFolder, status?: FolderStatus): DisplayState {
  const state: SyncState | null = status?.state ?? link.last_state;
  if (state === null) return link.paused ? 'paused' : 'unknown';
  const needsYou =
    state === 'conflict' ||
    state === 'missing_local_path' ||
    state === 'remote_missing' ||
    state === 'error';
  return link.paused && !needsYou ? 'paused' : state;
}

/** States the user should look at (the sidebar badge counts them). */
export function needsAttention(state: DisplayState): boolean {
  return state !== 'in_sync' && state !== 'unknown' && state !== 'paused';
}

export type PrimaryAction =
  | 'upload'
  | 'download'
  | 'resolve'
  | 'relink'
  | 'resume'
  | 'check'
  | 'history'
  | 'retry'
  | 'unlink';

/** The one action a state offers first. */
export function primaryAction(state: DisplayState): PrimaryAction {
  switch (state) {
    case 'local_changes':
      return 'upload';
    case 'remote_changes':
      return 'download';
    case 'conflict':
      return 'resolve';
    case 'server_conflict':
      return 'history';
    case 'missing_local_path':
      return 'relink';
    case 'remote_missing':
      return 'unlink';
    case 'error':
      return 'retry';
    case 'paused':
      return 'resume';
    case 'in_sync':
    case 'unknown':
      return 'check';
  }
}

export const ACTION_LABEL: Record<PrimaryAction, MessageKey> = {
  upload: 'desktop.action.upload',
  download: 'desktop.action.download',
  resolve: 'desktop.action.resolve',
  relink: 'desktop.action.relink',
  resume: 'desktop.action.resume',
  check: 'desktop.action.check',
  history: 'desktop.action.history',
  retry: 'desktop.action.retry',
  unlink: 'desktop.action.unlink'
};

export interface ChangeCounts {
  added: number;
  modified: number;
  removed: number;
}

export function changeCounts(changes: readonly PathChange[]): ChangeCounts {
  const counts: ChangeCounts = { added: 0, modified: 0, removed: 0 };
  for (const { change } of changes) counts[change]++;
  return counts;
}

/** Number of distinct paths changed on either side. */
export function changedPaths(status: FolderStatus): number {
  const paths = new Set<string>([
    ...status.local_changes.map((c) => c.path),
    ...status.remote_changes.map((c) => c.path),
    ...status.conflicting
  ]);
  return paths.size;
}

export interface FileRow {
  path: string;
  /** This PC's change, if any. */
  local: PathChange['change'] | null;
  /** The server's change, if any. */
  remote: PathChange['change'] | null;
  /** Changed on both sides. */
  both: boolean;
}

/** The file-by-file list of a conflict: every path changed on either side. */
export function fileRows(status: FolderStatus): FileRow[] {
  const rows = new Map<string, FileRow>();
  const row = (path: string) => {
    let existing = rows.get(path);
    if (!existing) {
      existing = { path, local: null, remote: null, both: false };
      rows.set(path, existing);
    }
    return existing;
  };
  for (const c of status.local_changes) row(c.path).local = c.change;
  for (const c of status.remote_changes) row(c.path).remote = c.change;
  for (const path of status.conflicting) row(path).both = true;
  return [...rows.values()].sort((a, b) => a.path.localeCompare(b.path));
}
