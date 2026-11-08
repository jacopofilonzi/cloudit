// The signed-in user and the server's login configuration, loaded once at startup
// (`GET /auth/config` + `GET /me`) and updated by the auth pages. The root layout
// reads it to guard routes (`guard.ts`).
import type { AuthConfig, UserProfile } from '@cloudit/api-types';
import { ApiError, getAuthConfig, getMe } from './api';

type Status = 'loading' | 'ready' | 'error';

/** Answers of `GET /me` meaning "no usable session" rather than a failure. */
const SIGNED_OUT: ReadonlySet<string> = new Set([
  'unauthorized',
  'setup_required',
  'account_disabled',
  'account_pending',
  'account_rejected'
]);

class Session {
  status = $state<Status>('loading');
  config = $state<AuthConfig | null>(null);
  user = $state<UserProfile | null>(null);
  error = $state<unknown>(null);

  get signedIn(): boolean {
    return this.user !== null;
  }

  get setupRequired(): boolean {
    return this.config?.setup_required ?? false;
  }

  /** (Re)loads the configuration and the current user. */
  async load(): Promise<void> {
    this.status = this.config ? this.status : 'loading';
    try {
      const [config, user] = await Promise.all([getAuthConfig(), loadUser()]);
      this.config = config;
      this.user = user;
      this.error = null;
      this.status = 'ready';
    } catch (err) {
      this.error = err;
      this.status = 'error';
    }
  }

  /** Reloads only the login configuration (providers, registration mode). */
  async refreshConfig(): Promise<void> {
    this.config = await getAuthConfig();
  }

  /** After setup, login or a sign-up that signed in. */
  signedInAs(profile: UserProfile): void {
    this.user = profile;
    if (this.config) this.config = { ...this.config, setup_required: false };
  }

  /** After logout, or when the server says the session is gone. */
  signedOut(): void {
    this.user = null;
  }
}

async function loadUser(): Promise<UserProfile | null> {
  try {
    return await getMe();
  } catch (err) {
    if (err instanceof ApiError && SIGNED_OUT.has(err.code)) return null;
    throw err;
  }
}

export const session = new Session();
