import { describe, expect, it } from 'vitest';
import {
  absoluteUrl,
  isProviderId,
  offersUnlimited,
  parseIntIn,
  parseList,
  quotaModeOf,
  quotaUpdate,
  slugify,
  splitBytes,
  toBytes
} from './admin';

describe('quota modes', () => {
  it('reads the own quota of an account, whatever the unlimited switch', () => {
    expect(quotaModeOf({ quota_bytes: null })).toBe('default');
    expect(quotaModeOf({ quota_bytes: 5 })).toBe('custom');
  });
  it('offers unlimited only when the default is limited or already chosen', () => {
    expect(offersUnlimited(1000, false)).toBe(true);
    expect(offersUnlimited(null, false)).toBe(false);
    expect(offersUnlimited(null, true)).toBe(true);
  });
  it('sends the switch and the own quota together', () => {
    expect(quotaUpdate(false, 'default', 7)).toEqual({ quota_bytes: null, quota_unlimited: false });
    expect(quotaUpdate(false, 'custom', 7)).toEqual({ quota_bytes: 7, quota_unlimited: false });
    expect(quotaUpdate(true, 'custom', 7)).toEqual({ quota_bytes: 7, quota_unlimited: true });
    expect(quotaUpdate(true, 'default', null)).toEqual({
      quota_bytes: null,
      quota_unlimited: true
    });
  });
  it('refuses a bad size only when it would be in force', () => {
    expect(quotaUpdate(false, 'custom', null)).toBeNull();
    expect(quotaUpdate(true, 'custom', null)).toEqual({ quota_unlimited: true });
  });
});

describe('quota units', () => {
  it('converts decimal units', () => {
    expect(toBytes('5', 'GB')).toBe(5_000_000_000);
    expect(toBytes('1,5', 'TB')).toBe(1_500_000_000_000);
    expect(toBytes('0', 'GB')).toBeNull();
    expect(toBytes('abc', 'MB')).toBeNull();
  });
  it('splits into the largest unit', () => {
    expect(splitBytes(5_000_000_000)).toEqual({ amount: '5', unit: 'GB' });
    expect(splitBytes(2_500_000)).toEqual({ amount: '2.5', unit: 'MB' });
    expect(splitBytes(3_000_000_000_000)).toEqual({ amount: '3', unit: 'TB' });
  });
});

describe('absoluteUrl', () => {
  it('prefers the public URL and trims slashes', () => {
    expect(absoluteUrl('/invite/x', 'https://a.example/', 'http://b')).toBe(
      'https://a.example/invite/x'
    );
    expect(absoluteUrl('/invite/x', null, 'http://b:80')).toBe('http://b:80/invite/x');
  });
});

describe('provider helpers', () => {
  it('validates ids and slugs names', () => {
    expect(isProviderId('my-sso')).toBe(true);
    expect(isProviderId('A')).toBe(false);
    expect(isProviderId('has space')).toBe(false);
    expect(slugify(' My GitLab! ')).toBe('my-gitlab');
  });
  it('parses lists and integers', () => {
    expect(parseList('a, b\n\n c')).toEqual(['a', 'b', 'c']);
    expect(parseIntIn('20', 0, 100)).toBe(20);
    expect(parseIntIn('-1', 0, 100)).toBeNull();
    expect(parseIntIn('101', 0, 100)).toBeNull();
  });
});
