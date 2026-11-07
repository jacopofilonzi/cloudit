// The single entry point for every call to the cloudit server: base URL, JSON
// decoding and error codes live here and nowhere else. Models come from
// `@cloudit/api-types` (generated from Rust); never redeclare them in the UI.
import { resolve } from '$app/paths';
import type { ApiErrorBody, ErrorCode, HealthResponse } from '@cloudit/api-types';

/**
 * Root of the app as served (SvelteKit `paths.base` + `/`). The server exposes the
 * API next to the UI, so every API URL is built from here, never hard-coded.
 */
export const APP_ROOT: string = resolve('/');

/** Base of the versioned API, e.g. `/api/v1` (or `/<base>/api/v1`). */
export const API_BASE = `${APP_ROOT}api/v1`;

/** Server codes plus the client-side ones (no response / unreadable response). */
export type ClientErrorCode = ErrorCode | 'network' | 'unknown';

/** A failed API call. Show `t(errorMessageKey(err.code))`, never `message`. */
export class ApiError extends Error {
  readonly code: ClientErrorCode;
  readonly status: number | undefined;

  constructor(code: ClientErrorCode, message: string, status?: number) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

const SERVER_CODES: Record<ErrorCode, true> = {
  bad_request: true,
  not_found: true,
  unavailable: true,
  internal: true,
  unauthorized: true,
  forbidden: true,
  invalid_credentials: true,
  account_disabled: true,
  registration_closed: true,
  invite_invalid: true,
  token_invalid: true,
  email_taken: true,
  validation_failed: true,
  rate_limited: true,
  csrf_failed: true,
  setup_required: true,
  setup_already_done: true,
  setup_token_invalid: true,
  last_admin: true
};

function isErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== 'object' || value === null) return false;
  const { code, message } = value as Record<string, unknown>;
  return typeof code === 'string' && code in SERVER_CODES && typeof message === 'string';
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(url, {
      ...init,
      headers: { Accept: 'application/json', ...init?.headers }
    });
  } catch (cause) {
    throw new ApiError('network', `fetch ${url} failed: ${String(cause)}`);
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    throw new ApiError('unknown', `invalid JSON from ${url}`, response.status);
  }

  if (!response.ok) {
    if (isErrorBody(body)) throw new ApiError(body.code, body.message, response.status);
    throw new ApiError('unknown', `HTTP ${response.status} from ${url}`, response.status);
  }
  return body as T;
}

/** `GET /api/v1/health`: server liveness and version. */
export function getHealth(): Promise<HealthResponse> {
  return request<HealthResponse>(`${API_BASE}/health`);
}
