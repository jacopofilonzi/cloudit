// The single entry point for every call to the cloudit server: base URL, credentials,
// JSON encoding and error codes live here and nowhere else. Models come from
// `@cloudit/api-types` (generated from Rust); never redeclare them in the UI.
//
// Every call sends the session cookie (`credentials: 'same-origin'`); the browser adds
// `Origin` to unsafe requests, which is what the server's CSRF check wants. Failures
// throw `ApiError` with the server's `code` (or `network` / `unknown`).
import { resolve } from '$app/paths';
import type {
  AddMemberRequest,
  AdminSummary,
  AdminUser,
  AdminUserList,
  AdminUserUpdate,
  ApiErrorBody,
  ApprovalStatus,
  AuthConfig,
  ChangePasswordRequest,
  CreateFolderRequest,
  CreateInviteRequest,
  CreateProviderRequest,
  CreateScopeRequest,
  DeviceAuthorizeRequest,
  DeviceAuthorizeResponse,
  DeviceInfo,
  ErrorCode,
  FolderInfo,
  HealthResponse,
  IdentityInfo,
  InviteInfo,
  InviteLink,
  InvitePreview,
  JobInfo,
  JobName,
  LoginRequest,
  Manifest,
  OAuthStartRequest,
  OAuthStartResponse,
  OnboardingCreateRequest,
  OnboardingInfo,
  OnboardingLinkRequest,
  PasswordResetLink,
  PasswordResetRequest,
  ProviderInfo,
  ProviderPreset,
  ProviderTestResult,
  RegisterRequest,
  RestoreRequest,
  RestoreSnapshotRequest,
  ScopeInfo,
  ScopeMember,
  ServerSettings,
  SessionInfo,
  SetupRequest,
  SetupStatus,
  SnapshotInfo,
  SnapshotKind,
  SnapshotList,
  SyncHeads,
  TrashEmptyResponse,
  TrashItem,
  UpdateDeviceRequest,
  UpdateFolderRequest,
  UpdateMeRequest,
  UpdateMemberRequest,
  UpdateProviderRequest,
  UpdateScopeRequest,
  UpdateSettingsRequest,
  UpdateSnapshotRequest,
  UsageInfo,
  UserProfile
} from '@cloudit/api-types';

/**
 * Root of the app as served (SvelteKit `paths.base` + `/`). The server exposes the
 * API next to the UI, so every API URL is built from here, never hard-coded.
 */
export const APP_ROOT: string = resolve('');

/** Base of the versioned API, e.g. `/api/v1` (or `/<base>/api/v1`). */
export const API_BASE = `${APP_ROOT}api/v1`;

/** Server codes plus the client-side ones (no response / unreadable response). */
export type ClientErrorCode = ErrorCode | 'network' | 'unknown';

/** A failed API call. Show `t(errorMessageKey(err))`, never `message`. */
export class ApiError extends Error {
  readonly code: ClientErrorCode;
  readonly status: number | undefined;
  /** With `validation_failed`: the request field at fault. */
  readonly field: string | undefined;
  /** With `conflict`: the folder's current head. */
  readonly headSnapshotId: string | undefined;
  /** With `chunk_missing` / `chunk_invalid`. */
  readonly hashes: readonly string[] | undefined;
  /** With `manifest_invalid`: the core error code. */
  readonly detail: string | undefined;
  /** With `rate_limited`: seconds to wait (`Retry-After`). */
  readonly retryAfter: number | undefined;

  constructor(
    code: ClientErrorCode,
    message: string,
    options: {
      status?: number;
      body?: ApiErrorBody;
      retryAfter?: number;
    } = {}
  ) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = options.status;
    this.field = options.body?.field;
    this.headSnapshotId = options.body?.head_snapshot_id;
    this.hashes = options.body?.hashes;
    this.detail = options.body?.detail;
    this.retryAfter = options.retryAfter;
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
  invite_invalid: true,
  token_invalid: true,
  email_taken: true,
  validation_failed: true,
  rate_limited: true,
  csrf_failed: true,
  setup_required: true,
  setup_already_done: true,
  setup_token_invalid: true,
  last_admin: true,
  provider_not_found: true,
  provider_id_taken: true,
  provider_discovery_failed: true,
  provider_error: true,
  public_url_required: true,
  provider_disabled: true,
  oauth_state_invalid: true,
  oauth_cancelled: true,
  group_not_allowed: true,
  identity_in_use: true,
  identity_not_linked: true,
  onboarding_expired: true,
  last_login_method: true,
  account_pending: true,
  account_rejected: true,
  name_taken: true,
  user_not_found: true,
  conflict: true,
  chunk_missing: true,
  chunk_invalid: true,
  manifest_invalid: true,
  quota_exceeded: true,
  payload_too_large: true
};

/** Whether `code` is a known server error code (e.g. from `/login?error=<code>`). */
export function isServerCode(code: unknown): code is ErrorCode {
  return typeof code === 'string' && Object.hasOwn(SERVER_CODES, code);
}

function isErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== 'object' || value === null) return false;
  const { code, message } = value as Record<string, unknown>;
  return isServerCode(code) && typeof message === 'string';
}

type Method = 'GET' | 'POST' | 'PATCH' | 'DELETE';
type Query = Record<string, string | number | boolean | undefined>;

interface Answer<T> {
  status: number;
  body: T;
  headers: Headers;
}

function url(path: string, query?: Query): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined) params.set(key, String(value));
  }
  const search = params.toString();
  return `${API_BASE}${path}${search ? `?${search}` : ''}`;
}

/** Path segment from an id or name (never trusted to be URL-safe). */
const seg = encodeURIComponent;

async function send<T>(
  method: Method,
  path: string,
  options: { body?: unknown; query?: Query; headers?: Record<string, string> } = {}
): Promise<Answer<T>> {
  const target = url(path, options.query);
  const headers: Record<string, string> = { Accept: 'application/json', ...options.headers };
  let body: string | undefined;
  if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(options.body);
  }
  let response: Response;
  try {
    response = await fetch(target, { method, headers, body, credentials: 'same-origin' });
  } catch (cause) {
    throw new ApiError('network', `${method} ${target} failed: ${String(cause)}`);
  }

  if (response.status === 304) {
    return { status: 304, body: undefined as T, headers: response.headers };
  }
  const text = await response.text().catch(() => '');
  let parsed: unknown = undefined;
  if (text) {
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new ApiError('unknown', `invalid JSON from ${method} ${target}`, {
        status: response.status
      });
    }
  }

  if (!response.ok) {
    const retry = Number(response.headers.get('Retry-After'));
    const retryAfter = Number.isFinite(retry) && retry > 0 ? retry : undefined;
    if (isErrorBody(parsed)) {
      throw new ApiError(parsed.code, parsed.message, {
        status: response.status,
        body: parsed,
        retryAfter
      });
    }
    throw new ApiError('unknown', `HTTP ${response.status} from ${method} ${target}`, {
      status: response.status
    });
  }
  return { status: response.status, body: parsed as T, headers: response.headers };
}

async function call<T>(method: Method, path: string, body?: unknown, query?: Query): Promise<T> {
  return (await send<T>(method, path, { body, query })).body;
}

const get = <T>(path: string, query?: Query) => call<T>('GET', path, undefined, query);
const post = <T>(path: string, body?: unknown) => call<T>('POST', path, body);
const patch = <T>(path: string, body: unknown) => call<T>('PATCH', path, body);
const del = (path: string) => call<void>('DELETE', path);

/** A sign-up either signs in (201) or waits for an admin's approval (202). */
export type SignupResult = { kind: 'signed_in'; profile: UserProfile } | { kind: 'pending' };

async function signup(path: string, body: unknown): Promise<SignupResult> {
  const answer = await send<unknown>('POST', path, { body });
  return answer.status === 202
    ? { kind: 'pending' }
    : { kind: 'signed_in', profile: answer.body as UserProfile };
}

// Probes and setup

/** `GET /health`: server liveness and version. */
export const getHealth = () => get<HealthResponse>('/health');
export const getSetupStatus = () => get<SetupStatus>('/setup');
/** `POST /setup` → the first admin, signed in. */
export const completeSetup = (body: SetupRequest) => post<UserProfile>('/setup', body);

// Authentication

export const getAuthConfig = () => get<AuthConfig>('/auth/config');
export const register = (body: RegisterRequest) => signup('/auth/register', body);
export const login = (body: LoginRequest) => post<UserProfile>('/auth/login', body);
export const logout = () => post<void>('/auth/logout');
export const resetPassword = (body: PasswordResetRequest) =>
  post<void>('/auth/password-reset', body);
export const lookupInvite = (token: string) =>
  post<InvitePreview>('/auth/invites/lookup', { token });
/** Starts a provider sign-in; navigate to `redirect_url`. */
export const startOAuth = (provider: string, body: OAuthStartRequest) =>
  post<OAuthStartResponse>(`/auth/oauth/${seg(provider)}/start`, body);

// Onboarding (after a provider sign-in with an unknown identity)

export const getOnboarding = () => get<OnboardingInfo>('/auth/onboarding');
export const onboardingCreate = (body: OnboardingCreateRequest) =>
  signup('/auth/onboarding/create', body);
export const onboardingLink = (body: OnboardingLinkRequest) =>
  post<UserProfile>('/auth/onboarding/link', body);
export const onboardingLinkStart = (provider: string) =>
  post<OAuthStartResponse>(`/auth/onboarding/link/${seg(provider)}/start`);

// The signed-in user

export const getMe = () => get<UserProfile>('/me');
export const updateMe = (body: UpdateMeRequest) => patch<UserProfile>('/me', body);
export const changePassword = (body: ChangePasswordRequest) => post<void>('/me/password', body);
export const listSessions = () => get<SessionInfo[]>('/me/sessions');
export const deleteSession = (id: string) => del(`/me/sessions/${seg(id)}`);
export const listIdentities = () => get<IdentityInfo[]>('/me/identities');
export const unlinkIdentity = (id: string) => del(`/me/identities/${seg(id)}`);
export const getUsage = () => get<UsageInfo>('/me/usage');

// Devices

export const authorizeDevice = (body: DeviceAuthorizeRequest) =>
  post<DeviceAuthorizeResponse>('/devices/authorize', body);
export const listDevices = () => get<DeviceInfo[]>('/devices');
export const updateDevice = (id: string, body: UpdateDeviceRequest) =>
  patch<DeviceInfo>(`/devices/${seg(id)}`, body);
export const revokeDevice = (id: string) => del(`/devices/${seg(id)}`);

// Scopes and members

export const listScopes = () => get<ScopeInfo[]>('/scopes');
export const createScope = (body: CreateScopeRequest) => post<ScopeInfo>('/scopes', body);
export const getScope = (id: string) => get<ScopeInfo>(`/scopes/${seg(id)}`);
export const updateScope = (id: string, body: UpdateScopeRequest) =>
  patch<ScopeInfo>(`/scopes/${seg(id)}`, body);
export const trashScope = (id: string) => del(`/scopes/${seg(id)}`);
export const restoreScope = (id: string, body: RestoreRequest = {}) =>
  post<ScopeInfo>(`/scopes/${seg(id)}/restore`, body);
export const listMembers = (scope: string) => get<ScopeMember[]>(`/scopes/${seg(scope)}/members`);
/** Adds a member, or changes the role of an existing one. */
export const addMember = (scope: string, body: AddMemberRequest) =>
  post<ScopeMember>(`/scopes/${seg(scope)}/members`, body);
export const updateMember = (scope: string, user: string, body: UpdateMemberRequest) =>
  patch<ScopeMember>(`/scopes/${seg(scope)}/members/${seg(user)}`, body);
/** Removes a member (the owner) or leaves the scope (the member itself). */
export const removeMember = (scope: string, user: string) =>
  del(`/scopes/${seg(scope)}/members/${seg(user)}`);

// Folders

export const listFolders = (scope: string) => get<FolderInfo[]>(`/scopes/${seg(scope)}/folders`);
export const createFolder = (scope: string, body: CreateFolderRequest) =>
  post<FolderInfo>(`/scopes/${seg(scope)}/folders`, body);
export const getFolder = (id: string) => get<FolderInfo>(`/folders/${seg(id)}`);
export const updateFolder = (id: string, body: UpdateFolderRequest) =>
  patch<FolderInfo>(`/folders/${seg(id)}`, body);
export const trashFolder = (id: string) => del(`/folders/${seg(id)}`);
export const restoreFolder = (id: string, body: RestoreRequest = {}) =>
  post<FolderInfo>(`/folders/${seg(id)}/restore`, body);

// Snapshots

export interface SnapshotQuery {
  cursor?: string;
  limit?: number;
  kind?: SnapshotKind;
  unresolved?: boolean;
}

export const listSnapshots = (folder: string, query: SnapshotQuery = {}) =>
  get<SnapshotList>(`/folders/${seg(folder)}/snapshots`, { ...query });
export const getSnapshot = (id: string) => get<SnapshotInfo>(`/snapshots/${seg(id)}`);
export const updateSnapshot = (id: string, body: UpdateSnapshotRequest) =>
  patch<SnapshotInfo>(`/snapshots/${seg(id)}`, body);
/** The snapshot's file list (canonical JSON). */
export const getManifest = (id: string) =>
  get<Manifest>(`/snapshots/${seg(id)}/manifest`, { format: 'json' });
/** "Keep this version": a restore snapshot becomes the head. */
export const restoreSnapshot = (id: string, body: RestoreSnapshotRequest = {}) =>
  post<SnapshotInfo>(`/snapshots/${seg(id)}/restore`, body);
/** "Keep the server's version" for a conflict snapshot. */
export const resolveSnapshot = (id: string) => post<SnapshotInfo>(`/snapshots/${seg(id)}/resolve`);
/** URL of the ZIP download (a plain link: the browser streams it). */
export const archiveUrl = (id: string) => url(`/snapshots/${seg(id)}/archive`);

// Trash

export const listTrash = () => get<TrashItem[]>('/trash');
export const purgeTrashedScope = (id: string) => del(`/trash/scopes/${seg(id)}`);
export const purgeTrashedFolder = (id: string) => del(`/trash/folders/${seg(id)}`);
export const emptyTrash = () => post<TrashEmptyResponse>('/trash/empty');

// Real-time

/** URL of the Server-Sent Events stream (`new EventSource(eventsUrl())`). */
export const eventsUrl = () => url('/events');
/** The heads of every visible folder and the `ETag` to send next time. */
export interface SyncHeadsAnswer {
  etag: string | null;
  heads: SyncHeads;
}

/**
 * `GET /sync/heads`; with the `etag` of the previous answer, `null` means nothing
 * changed (304).
 */
export async function getSyncHeads(etag?: string | null): Promise<SyncHeadsAnswer | null> {
  const answer = await send<SyncHeads>('GET', '/sync/heads', {
    headers: etag ? { 'If-None-Match': etag } : undefined
  });
  if (answer.status === 304) return null;
  return { etag: answer.headers.get('ETag'), heads: answer.body };
}

// Administration

export interface UserListQuery {
  limit?: number;
  offset?: number;
  approval_status?: ApprovalStatus;
}

export const listUsers = (query: UserListQuery = {}) =>
  get<AdminUserList>('/admin/users', { ...query });
export const updateUser = (id: string, body: AdminUserUpdate) =>
  patch<AdminUser>(`/admin/users/${seg(id)}`, body);
export const approveUser = (id: string) => post<AdminUser>(`/admin/users/${seg(id)}/approve`);
export const rejectUser = (id: string) => post<AdminUser>(`/admin/users/${seg(id)}/reject`);
export const createPasswordReset = (id: string) =>
  post<PasswordResetLink>(`/admin/users/${seg(id)}/password-reset`);
export const getAdminSummary = () => get<AdminSummary>('/admin/summary');
export const listInvites = () => get<InviteInfo[]>('/admin/invites');
export const createInvite = (body: CreateInviteRequest) => post<InviteLink>('/admin/invites', body);
export const revokeInvite = (id: string) => del(`/admin/invites/${seg(id)}`);
export const getSettings = () => get<ServerSettings>('/admin/settings');
export const updateSettings = (body: UpdateSettingsRequest) =>
  patch<ServerSettings>('/admin/settings', body);
export const listProviderPresets = () => get<ProviderPreset[]>('/admin/providers/presets');
export const listProviders = () => get<ProviderInfo[]>('/admin/providers');
export const createProvider = (body: CreateProviderRequest) =>
  post<ProviderInfo>('/admin/providers', body);
export const updateProvider = (id: string, body: UpdateProviderRequest) =>
  patch<ProviderInfo>(`/admin/providers/${seg(id)}`, body);
export const deleteProvider = (id: string) => del(`/admin/providers/${seg(id)}`);
export const testProvider = (id: string) =>
  post<ProviderTestResult>(`/admin/providers/${seg(id)}/test`);
export const listJobs = () => get<JobInfo[]>('/admin/jobs');
export const runJob = (name: JobName) => post<JobInfo>(`/admin/jobs/${seg(name)}/run`);

/**
 * The redirect URI to register at an identity provider, as the server derives it
 * from its public URL (shown before the provider exists, so the admin can register it).
 */
export const callbackUrl = (publicUrl: string, providerId: string) =>
  `${publicUrl.replace(/\/+$/u, '')}${url(`/auth/oauth/${seg(providerId)}/callback`)}`;
