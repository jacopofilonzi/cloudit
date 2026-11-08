import { afterEach, describe, expect, it as test, vi } from 'vitest';
import { avatarColorIndex, AVATAR_COLORS, initials } from './avatar';
import { answerConfirm, confirmAction, getPendingConfirm } from './confirm.svelte';
import { isThemeMode, resolveTheme } from './theme.svelte';
import { dismissToast, getToasts, showToast, toast } from './toast.svelte';

describe('theme', () => {
  test('resolves the choice against the system preference', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme('dark', false)).toBe('dark');
    expect(isThemeMode('dark')).toBe(true);
    expect(isThemeMode('blue')).toBe(false);
  });
});

describe('toasts', () => {
  afterEach(() => {
    for (const item of [...getToasts()]) dismissToast(item.id);
    vi.useRealTimers();
  });

  test('show and expire', () => {
    vi.useFakeTimers();
    const id = toast.success('Saved');
    const sticky = showToast('info', 'Sticky', 0);
    expect(getToasts().map((t) => t.message)).toEqual(['Saved', 'Sticky']);
    vi.advanceTimersByTime(5000);
    expect(getToasts().map((t) => t.id)).toEqual([sticky]);
    dismissToast(sticky);
    dismissToast(id);
    expect(getToasts()).toEqual([]);
  });

  test('errors stay longer', () => {
    vi.useFakeTimers();
    toast.error('Failed');
    vi.advanceTimersByTime(5000);
    expect(getToasts()).toHaveLength(1);
    vi.advanceTimersByTime(3000);
    expect(getToasts()).toHaveLength(0);
  });
});

describe('confirm', () => {
  test('resolves with the answer', async () => {
    const answer = confirmAction({ title: 'T', message: 'M', confirmLabel: 'OK' });
    expect(getPendingConfirm()?.title).toBe('T');
    answerConfirm(true);
    await expect(answer).resolves.toBe(true);
    expect(getPendingConfirm()).toBeNull();
  });

  test('a new request cancels the previous one', async () => {
    const first = confirmAction({ title: 'A', message: '', confirmLabel: 'OK' });
    const second = confirmAction({ title: 'B', message: '', confirmLabel: 'OK' });
    await expect(first).resolves.toBe(false);
    answerConfirm(false);
    await expect(second).resolves.toBe(false);
  });
});

describe('avatar', () => {
  test('initials', () => {
    expect(initials('Jacopo')).toBe('J');
    expect(initials('Sara Rossi')).toBe('SR');
    expect(initials('marco.bianchi@example.com')).toBe('MB');
    expect(initials('  ')).toBe('');
    expect(initials('élodie')).toBe('É');
  });

  test('colours are stable and in range', () => {
    const index = avatarColorIndex('user-1');
    expect(avatarColorIndex('user-1')).toBe(index);
    expect(index).toBeGreaterThanOrEqual(0);
    expect(index).toBeLessThan(AVATAR_COLORS);
  });
});
