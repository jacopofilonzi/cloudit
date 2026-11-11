// "Check now" for server updates: the server runs the check in the background
// (`POST /admin/update-check` answers at once), so the UI reads the summary again until
// the check is no longer pending, then tells what it found.
import type { AdminSummary } from '@cloudit/api-types';

export type CheckResult = 'newer' | 'up_to_date' | 'failed' | 'slow' | 'disabled';

/** What a summary says about a check that is no longer pending. */
export function checkResult(summary: AdminSummary): CheckResult {
  const check = summary.update_check;
  if (!check.enabled) return 'disabled';
  if (check.pending) return 'slow';
  if (check.last_outcome !== 'succeeded') return 'failed';
  return summary.update_available ? 'newer' : 'up_to_date';
}

/**
 * Starts from the summary `start` answered and reads `load` every `intervalMs` while a
 * check is pending, at most `tries` times. Calls `seen` with every summary read (so the
 * page shows it at once) and returns the last one.
 */
export async function waitForCheck(
  start: AdminSummary,
  load: () => Promise<AdminSummary>,
  seen: (summary: AdminSummary) => void,
  { intervalMs = 1500, tries = 20, sleep = defaultSleep } = {}
): Promise<AdminSummary> {
  let summary = start;
  seen(summary);
  for (let i = 0; i < tries && summary.update_check.pending; i++) {
    await sleep(intervalMs);
    summary = await load();
    seen(summary);
  }
  return summary;
}

const defaultSleep = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));
