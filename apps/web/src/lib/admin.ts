// Pure helpers of the administration pages (units, links, list fields).

import type { AdminUser, AdminUserUpdate } from '@cloudit/api-types';

/** An account's quota choice: the server default, no limit, or its own size. */
export type QuotaMode = 'default' | 'unlimited' | 'custom';

export function quotaModeOf(user: Pick<AdminUser, 'quota_bytes' | 'quota_unlimited'>): QuotaMode {
  if (user.quota_unlimited) return 'unlimited';
  return user.quota_bytes === null ? 'default' : 'custom';
}

/**
 * Whether "Unlimited" is offered: only when the server default is limited (otherwise
 * the default already means unlimited), or when the account already is unlimited.
 */
export function offersUnlimited(defaultQuota: number | null, mode: QuotaMode): boolean {
  return defaultQuota !== null || mode === 'unlimited';
}

/** The `PATCH /admin/users/{id}` body of a quota choice (`bytes` only for `custom`). */
export function quotaUpdate(
  mode: QuotaMode,
  bytes: number | null
): Pick<AdminUserUpdate, 'quota_bytes' | 'quota_unlimited'> {
  if (mode === 'unlimited') return { quota_bytes: null, quota_unlimited: true };
  return { quota_bytes: mode === 'custom' ? bytes : null, quota_unlimited: false };
}

export const QUOTA_UNITS = ['MB', 'GB', 'TB'] as const;
export type QuotaUnit = (typeof QUOTA_UNITS)[number];

const FACTOR: Record<QuotaUnit, number> = { MB: 1e6, GB: 1e9, TB: 1e12 };

/** Bytes of `amount` in `unit` (decimal, like the sizes the UI shows), or `null` if invalid. */
export function toBytes(amount: string | number, unit: QuotaUnit): number | null {
  const value = typeof amount === 'number' ? amount : Number(amount.trim().replace(',', '.'));
  if (!Number.isFinite(value) || value <= 0) return null;
  const bytes = Math.round(value * FACTOR[unit]);
  return Number.isSafeInteger(bytes) ? bytes : null;
}

/** The largest unit that shows `bytes` as a whole-ish number ("5000000000" is 5 GB). */
export function splitBytes(bytes: number): { amount: string; unit: QuotaUnit } {
  const unit = [...QUOTA_UNITS].reverse().find((u) => bytes >= FACTOR[u]) ?? 'MB';
  const amount = Math.round((bytes / FACTOR[unit]) * 100) / 100;
  return { amount: String(amount), unit };
}

/**
 * The absolute URL of a server path (`/invite/<token>`): the effective public URL
 * if the server knows one, else the origin the browser uses.
 */
export function absoluteUrl(path: string, publicUrl: string | null, origin: string): string {
  const base = (publicUrl ?? origin).replace(/\/+$/u, '');
  return `${base}${path}`;
}

/** Provider ids: 2..32 of lowercase letters, digits and dashes (the server's rule). */
export function isProviderId(id: string): boolean {
  return /^[a-z0-9-]{2,32}$/u.test(id);
}

/** A suggested provider id from a display name ("My GitLab" gives "my-gitlab"). */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/gu, '-')
    .replace(/^-+|-+$/gu, '')
    .slice(0, 32);
}

/** Comma or line separated values, trimmed, empty ones dropped. */
export function parseList(text: string): string[] {
  return text
    .split(/[\n,]/u)
    .map((item) => item.trim())
    .filter(Boolean);
}

/** The reverse of `parseList`, one value per line. */
export function formatList(items: readonly string[]): string {
  return items.join('\n');
}

/** Whole non-negative integer in `[min, max]`, or `null`. */
export function parseIntIn(text: string, min: number, max: number): number | null {
  const value = text.trim();
  if (!/^\d+$/u.test(value)) return null;
  const n = Number(value);
  return n >= min && n <= max ? n : null;
}
