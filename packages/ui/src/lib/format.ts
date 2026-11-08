// Locale-aware formatting of dates, durations and sizes. Every function takes the
// language explicitly (callers pass `getLang()`), so they stay pure and testable;
// templates re-render when the language changes because `getLang()` is reactive.
import type { Lang } from './i18n/i18n.svelte';

const LOCALES: Record<Lang, string> = { en: 'en-GB', it: 'it-IT' };

/** The `Intl` locale of a UI language. */
export function locale(lang: Lang): string {
  return LOCALES[lang];
}

type DateInput = Date | string | number;

function toDate(value: DateInput): Date {
  return value instanceof Date ? value : new Date(value);
}

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/**
 * "3 minutes ago", "in 6 days", "yesterday": the largest unit that fits. Under a
 * minute reads as "now". Months and years are approximated (30 / 365 days).
 */
export function formatRelative(value: DateInput, lang: Lang, now: DateInput = Date.now()): string {
  const diff = toDate(value).getTime() - toDate(now).getTime();
  const abs = Math.abs(diff);
  const rtf = new Intl.RelativeTimeFormat(locale(lang), { numeric: 'auto' });
  const at = (unit: Intl.RelativeTimeFormatUnit, size: number) =>
    rtf.format(Math.round(diff / size), unit);
  if (abs < MINUTE) return rtf.format(0, 'second');
  if (abs < HOUR) return at('minute', MINUTE);
  if (abs < DAY) return at('hour', HOUR);
  if (abs < 30 * DAY) return at('day', DAY);
  if (abs < 365 * DAY) return at('month', 30 * DAY);
  return at('year', 365 * DAY);
}

/** Date and time, e.g. "8 Nov 2026, 14:32" (local time zone). */
export function formatDateTime(value: DateInput, lang: Lang): string {
  return new Intl.DateTimeFormat(locale(lang), {
    dateStyle: 'medium',
    timeStyle: 'short'
  }).format(toDate(value));
}

/** Date only, e.g. "8 Nov 2026". */
export function formatDate(value: DateInput, lang: Lang): string {
  return new Intl.DateTimeFormat(locale(lang), { dateStyle: 'medium' }).format(toDate(value));
}

/** Time only, e.g. "14:32". */
export function formatTime(value: DateInput, lang: Lang): string {
  return new Intl.DateTimeFormat(locale(lang), { timeStyle: 'short' }).format(toDate(value));
}

const BYTE_UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte', 'petabyte'] as const;

/**
 * A byte count in decimal units ("1.2 GB", "1,2 GB"): one decimal below 10, none
 * above, never more than 3 significant digits.
 */
export function formatBytes(bytes: number, lang: Lang): string {
  let value = Math.max(0, bytes);
  // CLDR's short "byte" is a word in most languages; "B" matches kB, MB…
  if (value < 1000) return `${formatNumber(Math.round(value), lang)} B`;
  let unit = 0;
  while (value >= 1000 && unit < BYTE_UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return new Intl.NumberFormat(locale(lang), {
    style: 'unit',
    unit: BYTE_UNITS[unit],
    unitDisplay: 'short',
    maximumFractionDigits: unit === 0 || value >= 10 ? 0 : 1
  }).format(value);
}

/** A plain number with the language's grouping ("2,410" / "2.410"). */
export function formatNumber(value: number, lang: Lang): string {
  return new Intl.NumberFormat(locale(lang)).format(value);
}

/** The UTC calendar day of a timestamp, `YYYY-MM-DD` (retention counts UTC days). */
export function utcDayKey(value: DateInput): string {
  return toDate(value).toISOString().slice(0, 10);
}

/** Consecutive items grouped by UTC day, in their original order. */
export function groupByUtcDay<T>(
  items: readonly T[],
  dateOf: (item: T) => DateInput
): { day: string; items: T[] }[] {
  const groups: { day: string; items: T[] }[] = [];
  for (const item of items) {
    const day = utcDayKey(dateOf(item));
    const last = groups.at(-1);
    if (last?.day === day) last.items.push(item);
    else groups.push({ day, items: [item] });
  }
  return groups;
}

/** Label of a UTC day key: "today", "yesterday", else a short date ("3 Nov"). */
export function formatUtcDay(day: string, lang: Lang, now: DateInput = Date.now()): string {
  const start = Date.parse(`${day}T00:00:00Z`);
  const today = Date.parse(`${utcDayKey(now)}T00:00:00Z`);
  const days = Math.round((start - today) / DAY);
  if (days === 0 || days === -1) {
    const text = new Intl.RelativeTimeFormat(locale(lang), { numeric: 'auto' }).format(days, 'day');
    return text.charAt(0).toLocaleUpperCase(locale(lang)) + text.slice(1);
  }
  const sameYear = day.slice(0, 4) === utcDayKey(now).slice(0, 4);
  return new Intl.DateTimeFormat(locale(lang), {
    day: 'numeric',
    month: 'short',
    year: sameYear ? undefined : 'numeric',
    timeZone: 'UTC'
  }).format(start);
}
