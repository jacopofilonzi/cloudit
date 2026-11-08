// The `/device` approval page's query contract with the desktop app (RFC 8252
// loopback + PKCE). The app opens:
//
//   /device?code_challenge=<43 base64url>&code_challenge_method=S256
//          &redirect_uri=http://127.0.0.1:<port>/<path>&state=<opaque>
//          &device_name=<name>&os=<windows|linux|macos>
//
// Allow → `POST /devices/authorize`, then the browser goes to the returned
// `redirect_url` (`redirect_uri?code=…&state=…`). Cancel → the browser goes to
// `redirect_uri?error=access_denied&state=<state>`. The checks here mirror the
// server's so a broken link is reported before asking anything; the server checks
// again on Allow.
import type { DeviceAuthorizeRequest } from '@cloudit/api-types';

export type DeviceRequest = Omit<DeviceAuthorizeRequest, 'token_ttl'>;

const CHALLENGE = /^[A-Za-z0-9_-]{43}$/;

/** Whether `uri` is a loopback redirect the server accepts (explicit port, no query). */
export function isLoopbackRedirect(uri: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(uri);
  } catch {
    return false;
  }
  const hostOk = parsed.hostname === '127.0.0.1' || parsed.hostname === '[::1]';
  const explicitPort = /^http:\/\/(127\.0\.0\.1|\[::1\]):\d+(\/|$)/.test(uri);
  return (
    parsed.protocol === 'http:' &&
    hostOk &&
    explicitPort &&
    parsed.port !== '' &&
    !uri.includes('?') &&
    !uri.includes('#') &&
    parsed.username === '' &&
    parsed.password === ''
  );
}

/** The request in the page's query, or `null` when a parameter is missing or invalid. */
export function parseDeviceRequest(params: {
  get(name: string): string | null;
}): DeviceRequest | null {
  const challenge = params.get('code_challenge') ?? '';
  const method = params.get('code_challenge_method') ?? 'S256';
  const redirect = params.get('redirect_uri') ?? '';
  const state = params.get('state') ?? '';
  if (!CHALLENGE.test(challenge) || method !== 'S256') return null;
  if (!isLoopbackRedirect(redirect)) return null;
  if (state.length < 1 || state.length > 512) return null;
  return {
    code_challenge: challenge,
    code_challenge_method: 'S256',
    redirect_uri: redirect,
    state,
    device_name: (params.get('device_name') ?? '').trim(),
    os: (params.get('os') ?? '').trim()
  };
}

/** Where Cancel sends the browser: the app's redirect with `error=access_denied`. */
export function cancelUrl(request: DeviceRequest): string {
  const query = new URLSearchParams({ error: 'access_denied', state: request.state });
  return `${request.redirect_uri}?${query.toString()}`;
}
