import { describe, expect, it as test } from 'vitest';
import { cancelUrl, isLoopbackRedirect, parseDeviceRequest } from './device';

const CHALLENGE = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';

function params(extra: Record<string, string> = {}) {
  return new URLSearchParams({
    code_challenge: CHALLENGE,
    code_challenge_method: 'S256',
    redirect_uri: 'http://127.0.0.1:53682/callback',
    state: 'st4te',
    device_name: ' Laptop ',
    os: 'windows',
    ...extra
  });
}

describe('isLoopbackRedirect', () => {
  test('accepts loopback with an explicit port only', () => {
    expect(isLoopbackRedirect('http://127.0.0.1:53682/callback')).toBe(true);
    expect(isLoopbackRedirect('http://[::1]:8000/')).toBe(true);
    expect(isLoopbackRedirect('http://127.0.0.1:53682')).toBe(true);
    for (const bad of [
      'http://localhost:53682/',
      'https://127.0.0.1:53682/',
      'http://127.0.0.1/',
      'http://127.0.0.1:53682/?a=1',
      'http://127.0.0.1:53682/#x',
      'http://evil.example:53682/',
      'http://u:p@127.0.0.1:53682/',
      'not a url'
    ]) {
      expect(isLoopbackRedirect(bad), bad).toBe(false);
    }
  });
});

describe('parseDeviceRequest', () => {
  test('reads a valid request', () => {
    expect(parseDeviceRequest(params())).toEqual({
      code_challenge: CHALLENGE,
      code_challenge_method: 'S256',
      redirect_uri: 'http://127.0.0.1:53682/callback',
      state: 'st4te',
      device_name: 'Laptop',
      os: 'windows'
    });
    expect(parseDeviceRequest(params({ code_challenge_method: '' }))).toBeNull();
  });

  test('refuses broken requests', () => {
    expect(parseDeviceRequest(params({ code_challenge: 'short' }))).toBeNull();
    expect(parseDeviceRequest(params({ code_challenge_method: 'plain' }))).toBeNull();
    expect(parseDeviceRequest(params({ redirect_uri: 'https://evil.example/' }))).toBeNull();
    expect(parseDeviceRequest(params({ state: '' }))).toBeNull();
    expect(parseDeviceRequest(params({ state: 'x'.repeat(513) }))).toBeNull();
  });

  test('cancel URL', () => {
    const request = parseDeviceRequest(params({ state: 'a b&c' }));
    expect(request && cancelUrl(request)).toBe(
      'http://127.0.0.1:53682/callback?error=access_denied&state=a+b%26c'
    );
  });
});
