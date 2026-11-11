// Pure rules of the app update UI: whether the signed-in shell is replaced, and what the
// "update required" screen offers for the updater's state (tested).
import type { Compat, UpdateState } from '@cloudit/api-types';

/** This app's version and the server's minimum. */
export type RequiredVersions = { appVersion: string; minVersion: string };

/**
 * The signed-in shell shows only the "update required" screen (with these versions)
 * while the server needs a newer app; `null`: the normal shell.
 */
export function updateRequired(compat: Compat): RequiredVersions | null {
  return compat.status === 'app_outdated'
    ? { appVersion: compat.app_version, minVersion: compat.min_client_version }
    : null;
}

/** What the "update required" screen shows for the updater's state. */
export type RequiredUpdate =
  /** A newer version can be installed from the app. */
  | { kind: 'install'; version: string }
  /** A newer version is out, installed with a package: the release page. */
  | { kind: 'download_page'; version: string }
  /** A development build: the version is shown, never installed. */
  | { kind: 'dev_build'; version: string }
  /** The install the user asked for is under way. */
  | {
      kind: 'waiting' | 'downloading' | 'installing';
      version: string;
      percent: number | null;
    }
  /** The install failed: try again. */
  | { kind: 'failed'; version: string }
  /** A check is running. */
  | { kind: 'checking' }
  /** No newer version known (never checked, none found, or the check failed). */
  | { kind: 'none'; checkFailed: boolean };

/** Download progress in percent, when the size is known. */
export function downloadPercent(update: UpdateState): number | null {
  return update.total ? Math.min(100, Math.round((update.downloaded / update.total) * 100)) : null;
}

export function requiredUpdate(update: UpdateState | null): RequiredUpdate {
  if (!update) return { kind: 'none', checkFailed: false };
  if (update.phase === 'checking') return { kind: 'checking' };
  const version = update.latest_version;
  if (!version) return { kind: 'none', checkFailed: update.phase === 'failed' };
  switch (update.phase) {
    case 'waiting':
    case 'downloading':
    case 'installing':
      return { kind: update.phase, version, percent: downloadPercent(update) };
    case 'failed':
      return { kind: 'failed', version };
    case 'available':
      if (update.install === 'in_app') return { kind: 'install', version };
      if (update.install === 'release_page') return { kind: 'download_page', version };
      return { kind: 'dev_build', version };
    default:
      return { kind: 'none', checkFailed: false };
  }
}

/** Entering the screen checks for a version at once when none is known yet. */
export function checkOnEntry(update: UpdateState | null): boolean {
  return !update || (!update.latest_version && update.phase !== 'checking');
}
