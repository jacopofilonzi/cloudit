import { describe, expect, it as test } from 'vitest';
import {
  formatBytes,
  formatNumber,
  formatRelative,
  formatUtcDay,
  groupByUtcDay,
  utcDayKey
} from './format';

const NOW = Date.parse('2026-11-08T14:00:00Z');
const ago = (ms: number) => NOW - ms;
const MIN = 60_000;

describe('formatBytes', () => {
  test('uses decimal units and the language separators', () => {
    expect(formatBytes(0, 'en')).toBe('0 B');
    expect(formatBytes(999, 'en')).toBe('999 B');
    expect(formatBytes(1_200_000_000, 'en')).toBe('1.2 GB');
    expect(formatBytes(1_200_000_000, 'it')).toBe('1,2 GB');
    expect(formatBytes(640_000_000, 'en')).toBe('640 MB');
    expect(formatBytes(88_400_000, 'it')).toBe('88 MB');
    expect(formatBytes(-5, 'en')).toBe('0 B');
    expect(formatBytes(1500, 'it')).toBe('1,5 kB');
  });

  test('groups plain numbers', () => {
    expect(formatNumber(2410, 'en')).toBe('2,410');
    expect(formatNumber(24100, 'it')).toBe('24.100');
  });
});

describe('formatRelative', () => {
  test('picks the largest unit', () => {
    expect(formatRelative(ago(10_000), 'en', NOW)).toBe('now');
    expect(formatRelative(ago(3 * MIN), 'en', NOW)).toBe('3 minutes ago');
    expect(formatRelative(ago(3 * MIN), 'it', NOW)).toBe('3 minuti fa');
    expect(formatRelative(ago(26 * 60 * MIN), 'en', NOW)).toBe('yesterday');
    expect(formatRelative(NOW + 6 * 24 * 60 * MIN, 'it', NOW)).toBe('tra 6 giorni');
    expect(formatRelative(NOW + 330 * 24 * 60 * MIN, 'en', NOW)).toBe('in 11 months');
  });
});

describe('UTC days', () => {
  test('keys and labels', () => {
    expect(utcDayKey('2026-11-08T23:59:59Z')).toBe('2026-11-08');
    expect(formatUtcDay('2026-11-08', 'it', NOW)).toBe('Oggi');
    expect(formatUtcDay('2026-11-07', 'en', NOW)).toBe('Yesterday');
    expect(formatUtcDay('2026-11-03', 'it', NOW)).toBe('3 nov');
    expect(formatUtcDay('2025-11-03', 'en', NOW)).toBe('3 Nov 2025');
  });

  test('groups consecutive items by day', () => {
    const items = [
      { at: '2026-11-08T14:32:00Z' },
      { at: '2026-11-08T11:05:00Z' },
      { at: '2026-11-07T22:14:00Z' },
      { at: '2026-11-03T21:02:00Z' }
    ];
    const groups = groupByUtcDay(items, (item) => item.at);
    expect(groups.map((g) => [g.day, g.items.length])).toEqual([
      ['2026-11-08', 2],
      ['2026-11-07', 1],
      ['2026-11-03', 1]
    ]);
    expect(groupByUtcDay([], () => 0)).toEqual([]);
  });
});
