import { describe, expect, it as test } from 'vitest';
import { inviteToken, osName } from './fields';

describe('inviteToken', () => {
  test('takes a token or a link', () => {
    expect(inviteToken(' abc_DEF-1 ')).toBe('abc_DEF-1');
    expect(inviteToken('https://sync.example.com/invite/abc_DEF-1')).toBe('abc_DEF-1');
    expect(inviteToken('/invite/abc?x=1')).toBe('abc');
  });
});

describe('osName', () => {
  test('names known systems', () => {
    expect(osName('windows')).toBe('Windows');
    expect(osName('Darwin')).toBe('macOS');
    expect(osName(' FreeBSD ')).toBe('FreeBSD');
  });
});
