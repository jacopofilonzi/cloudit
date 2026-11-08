import { describe, expect, it as test } from 'vitest';
import { guard, loginFor, safeReturnTo } from './guard';

const out = { setupRequired: false, signedIn: false };
const inn = { setupRequired: false, signedIn: true };
const setup = { setupRequired: true, signedIn: false };

describe('guard', () => {
  test('setup first', () => {
    expect(guard('/', '', setup)).toBe('/setup');
    expect(guard('/login', '', setup)).toBe('/setup');
    expect(guard('/setup', '', setup)).toBeNull();
    expect(guard('/setup', '', out)).toBe('/login');
    expect(guard('/setup', '', inn)).toBe('/');
  });

  test('app pages need a session', () => {
    expect(guard('/', '', out)).toBe('/login');
    expect(guard('/', '', inn)).toBeNull();
    expect(guard('/device', '?state=x&os=linux', out)).toBe(
      '/login?return_to=%2Fdevice%3Fstate%3Dx%26os%3Dlinux'
    );
    expect(guard('/scopes/1', '', out)).toBe('/login?return_to=%2Fscopes%2F1');
  });

  test('guest pages send signed-in users on', () => {
    expect(guard('/login', '', out)).toBeNull();
    expect(guard('/invite/abc', '', out)).toBeNull();
    expect(guard('/login', '', inn)).toBe('/');
    expect(guard('/register', '', inn)).toBe('/');
    expect(guard('/login', '?return_to=%2Fdevice%3Fa%3D1', inn)).toBe('/device?a=1');
    expect(guard('/login', '?return_to=https%3A%2F%2Fevil.example', inn)).toBe('/');
    expect(guard('/login', '?return_to=%2F%2Fevil.example', inn)).toBe('/');
  });

  test('open pages never redirect', () => {
    for (const route of ['/reset-password/tok', '/onboarding', '/pending']) {
      expect(guard(route, '', out)).toBeNull();
      expect(guard(route, '', inn)).toBeNull();
    }
    // Only exact matches: `/pendingx` is an app page.
    expect(guard('/pendingx', '', out)).toBe('/login?return_to=%2Fpendingx');
  });
});

describe('safeReturnTo', () => {
  test('keeps in-app paths only', () => {
    expect(safeReturnTo('/device?x=1')).toBe('/device?x=1');
    for (const bad of [null, '', 'device', '//evil', '/\\evil', '/a\nb', `/${'a'.repeat(1024)}`]) {
      expect(safeReturnTo(bad)).toBeUndefined();
    }
  });

  test('loginFor', () => {
    expect(loginFor('/')).toBe('/login');
    expect(loginFor('/', '?a=1')).toBe('/login?return_to=%2F%3Fa%3D1');
  });
});
