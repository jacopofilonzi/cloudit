// Text and fraction of a progress report. Pure: the language is passed in.
import type { Phase } from '@cloudit/api-types';
import { formatBytes, formatNumber } from '@cloudit/ui';
import { t, type Lang } from '@cloudit/ui/i18n';

/** What the operation is doing, in words. */
export function phaseText(phase: Phase, lang: Lang): string {
  switch (phase.phase) {
    case 'scanning':
      return t('desktop.phase.scanning', { files: formatNumber(phase.files, lang) });
    case 'checking':
      return t('desktop.phase.checking');
    case 'uploading':
      return t('desktop.phase.uploading', {
        done: formatBytes(phase.done_bytes, lang),
        total: formatBytes(phase.total_bytes, lang)
      });
    case 'committing':
      return t('desktop.phase.committing');
    case 'downloading':
      return t('desktop.phase.downloading', {
        done: formatBytes(phase.done_bytes, lang),
        total: formatBytes(phase.total_bytes, lang)
      });
    case 'staging':
      return t('desktop.phase.staging', { done: phase.done, total: phase.total });
    case 'applying':
      return t('desktop.phase.applying', { done: phase.done, total: phase.total });
  }
}

/** Progress of the phase from 0 to 1, or `null` when it has no known total. */
export function phaseFraction(phase: Phase): number | null {
  switch (phase.phase) {
    case 'uploading':
    case 'downloading':
      return phase.total_bytes > 0 ? Math.min(1, phase.done_bytes / phase.total_bytes) : null;
    case 'staging':
    case 'applying':
      return phase.total > 0 ? Math.min(1, phase.done / phase.total) : null;
    case 'scanning':
    case 'checking':
    case 'committing':
      return null;
  }
}
