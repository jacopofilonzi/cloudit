import { describe, expect, it } from 'vitest';
import type { AdminSummary, UpdateCheckStatus } from '@cloudit/api-types';
import { checkResult, waitForCheck } from './update-check.ts';

function summary(check: Partial<UpdateCheckStatus>, update_available = false): AdminSummary {
  return {
    pending_users: 0,
    server_version: '1.0.0',
    latest_release: null,
    update_available,
    update_check: {
      enabled: true,
      pending: false,
      last_outcome: 'succeeded',
      last_finished_at: '2026-11-11T10:00:00Z',
      ...check
    }
  };
}

describe('checkResult', () => {
  it('maps the finished check', () => {
    expect(checkResult(summary({}, true))).toBe('newer');
    expect(checkResult(summary({}))).toBe('up_to_date');
    expect(checkResult(summary({ last_outcome: 'failed' }))).toBe('failed');
    expect(checkResult(summary({ last_outcome: 'interrupted' }))).toBe('failed');
    expect(checkResult(summary({ last_outcome: null }))).toBe('failed');
    expect(checkResult(summary({ pending: true }))).toBe('slow');
    expect(checkResult(summary({ enabled: false, pending: true }))).toBe('disabled');
  });
});

describe('waitForCheck', () => {
  const sleep = async () => {};

  it('reads again while pending', async () => {
    const answers = [summary({ pending: true }), summary({}, true)];
    const seen: AdminSummary[] = [];
    const last = await waitForCheck(
      summary({ pending: true }),
      async () => answers.shift()!,
      (s) => seen.push(s),
      { sleep }
    );
    expect(checkResult(last)).toBe('newer');
    expect(seen).toHaveLength(3);
  });

  it('gives up after the last try', async () => {
    let loads = 0;
    const last = await waitForCheck(
      summary({ pending: true }),
      async () => {
        loads++;
        return summary({ pending: true });
      },
      () => {},
      { sleep, tries: 3 }
    );
    expect(loads).toBe(3);
    expect(checkResult(last)).toBe('slow');
  });

  it('does not read when nothing is pending', async () => {
    let loads = 0;
    await waitForCheck(
      summary({}),
      async () => {
        loads++;
        return summary({});
      },
      () => {},
      { sleep }
    );
    expect(loads).toBe(0);
  });
});
