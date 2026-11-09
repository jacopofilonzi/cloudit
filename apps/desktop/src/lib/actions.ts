// User-level actions shared by several screens: syncing a folder with its toast, and
// the navigation helpers (typed routes through `resolve`).
import type { LinkedFolder, SyncReport } from '@cloudit/api-types';
import { toast } from '@cloudit/ui';
import { t } from '@cloudit/ui/i18n';
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import * as api from './api';
import { app } from './app.svelte';
import { CommandError } from './api';
import { errorText } from './errors';
import { tn } from './plural';
import { displayState, primaryAction } from './status';

export function showFolders(): Promise<void> {
  return goto(resolve('/(app)/folders'));
}

export function showConflict(id: number): Promise<void> {
  return goto(resolve('/(app)/folders/[id]/conflict', { id: String(id) }));
}

export function showFolderSettings(id: number): Promise<void> {
  return goto(resolve('/(app)/folders/[id]/settings', { id: String(id) }));
}

/** Tells the user how a sync went; a conflict opens the decision screen. */
export function reportOutcome(link: LinkedFolder, report: SyncReport): void {
  const name = link.folder_name;
  switch (report.outcome.outcome) {
    case 'up_to_date':
      toast.info(t('desktop.result.up_to_date', { name }));
      break;
    case 'pushed':
      toast.success(t('desktop.result.pushed', { name }));
      break;
    case 'pulled':
      toast.success(tn('desktop.result.pulled', report.outcome.changes, { name }));
      break;
    case 'needs_decision':
      toast.info(t('desktop.result.needs_decision', { name }));
      void showConflict(link.id);
      break;
    case 'deferred':
      toast.info(t('desktop.result.deferred', { name }));
      break;
  }
}

/** Reports a failed command (a cancel is not an error). */
export function reportError(error: unknown): void {
  app.noteError(error);
  if (error instanceof CommandError && error.kind === 'cancelled') {
    toast.info(t('desktop.result.cancelled'));
  } else {
    toast.error(errorText(error));
  }
}

/** Syncs one folder and says how it went. */
export async function syncAndReport(link: LinkedFolder): Promise<SyncReport | null> {
  try {
    const report = await api.syncFolder(link.id);
    reportOutcome(link, report);
    return report;
  } catch (error) {
    reportError(error);
    return null;
  }
}

/** Does what the folder's main button says. */
export async function runPrimary(link: LinkedFolder): Promise<void> {
  const state = displayState(link, app.statuses[link.id]);
  try {
    switch (primaryAction(state)) {
      case 'resolve':
        await showConflict(link.id);
        break;
      case 'relink':
      case 'unlink':
        await showFolderSettings(link.id);
        break;
      case 'history':
        await api.openHistory(link.id);
        break;
      case 'resume':
        await api.setPaused(link.id, false);
        await app.loadFolders();
        await api.refreshStatus(link.id);
        break;
      case 'upload':
      case 'download':
      case 'check':
      case 'retry':
        await syncAndReport(link);
        break;
    }
  } catch (error) {
    reportError(error);
  }
}
