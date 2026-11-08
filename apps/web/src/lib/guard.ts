// Where the app sends the browser before showing a page, from the session state.
// Pure (no SvelteKit imports) so it is unit-tested; the root layout applies it.
//
// Routes are paths without the app's base path (`/login`, `/invite/abc`).
// - While first-run setup is pending, everything goes to `/setup` (and `/setup` is
//   left once it is done).
// - Guest pages (`/login`, `/register`, `/invite/…`) send a signed-in user on to
//   `return_to` (if safe) or `/`.
// - Open pages (`/reset-password/…`, `/onboarding`, `/pending`) never redirect.
// - Every other page (the app and `/device`) needs a session: signed-out users go to
//   `/login?return_to=<path and query>` (no `return_to` for `/`).

export interface GuardState {
  setupRequired: boolean;
  signedIn: boolean;
}

const GUEST = ['/login', '/register', '/invite/'];
const OPEN = ['/reset-password/', '/onboarding', '/pending'];

function matches(route: string, prefixes: readonly string[]): boolean {
  return prefixes.some((prefix) =>
    prefix.endsWith('/') ? route.startsWith(prefix) : route === prefix
  );
}

/**
 * A safe in-app path to return to after signing in: starts with one `/`, no
 * backslash or control characters, at most 1024 bytes (the server's `return_to`
 * rules). Anything else is dropped.
 */
export function safeReturnTo(value: string | null | undefined): string | undefined {
  if (!value || !value.startsWith('/') || value.startsWith('//')) return undefined;
  if (value.includes('\\') || [...value].some((c) => c <= '\u001f' || c === '\u007f')) {
    return undefined;
  }
  if (new TextEncoder().encode(value).length > 1024) return undefined;
  return value;
}

/** The login page that comes back to `route` + `search` afterwards. */
export function loginFor(route: string, search = ''): string {
  if (route === '/' && !search) return '/login';
  return `/login?${new URLSearchParams({ return_to: route + search }).toString()}`;
}

/** The route to redirect to, or `null` to show `route`. */
export function guard(route: string, search: string, state: GuardState): string | null {
  if (state.setupRequired) return route === '/setup' ? null : '/setup';
  if (route === '/setup') return state.signedIn ? '/' : '/login';
  if (matches(route, OPEN)) return null;
  if (matches(route, GUEST)) {
    if (!state.signedIn) return null;
    return safeReturnTo(new URLSearchParams(search).get('return_to')) ?? '/';
  }
  return state.signedIn ? null : loginFor(route, search);
}
