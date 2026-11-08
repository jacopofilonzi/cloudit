import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { coalesced } from './coalesce';

describe('coalesced', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('merges calls within the delay into one run', async () => {
    const run = vi.fn(async () => {});
    const refresh = coalesced(run, 100);
    refresh();
    refresh();
    refresh();
    await vi.advanceTimersByTimeAsync(100);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it('runs once more when called during a run', async () => {
    let release: () => void = () => {};
    const run = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          release = resolve;
        })
    );
    const refresh = coalesced(run, 10);
    refresh();
    await vi.advanceTimersByTimeAsync(10);
    expect(run).toHaveBeenCalledTimes(1);
    refresh();
    release();
    await vi.advanceTimersByTimeAsync(20);
    expect(run).toHaveBeenCalledTimes(2);
    release();
  });

  it('survives a failing run', async () => {
    const run = vi.fn(async () => {
      throw new Error('boom');
    });
    const refresh = coalesced(run, 10);
    refresh();
    await vi.advanceTimersByTimeAsync(10);
    refresh();
    await vi.advanceTimersByTimeAsync(10);
    expect(run).toHaveBeenCalledTimes(2);
  });
});
