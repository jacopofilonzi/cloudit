// The only module that talks to the Rust backend (Tauri commands). Every command
// failure becomes a `CommandError` carrying the engine's stable `ErrorKind`.
import { invoke } from '@tauri-apps/api/core';
import type {
  ActivityEntry,
  AppInfo,
  AppSettings,
  AppSettingsUpdate,
  AutoFolderState,
  AutoSyncSettings,
  CommandError as CommandErrorBody,
  ErrorDetails,
  ErrorKind,
  FolderInfo,
  FolderStatus,
  LinkedFolder,
  LoginStarted,
  ScopeInfo,
  SessionState,
  SyncReport,
  UpdateState,
  VersionInfo
} from '@cloudit/api-types';

/** A failed command: `kind` is translated by `errors.ts`, `message` is for logs. */
export class CommandError extends Error {
  readonly kind: ErrorKind;
  readonly details: ErrorDetails | null;

  constructor(body: CommandErrorBody) {
    super(body.message);
    this.name = 'CommandError';
    this.kind = body.kind;
    this.details = body.details;
  }
}

function isBody(value: unknown): value is CommandErrorBody {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { kind?: unknown }).kind === 'string' &&
    typeof (value as { message?: unknown }).message === 'string'
  );
}

/** Wraps whatever `invoke` rejected with (a body, or a string from Tauri itself). */
export function toCommandError(error: unknown): CommandError {
  if (error instanceof CommandError) return error;
  if (isBody(error)) return new CommandError({ ...error, details: error.details ?? null });
  return new CommandError({
    kind: 'state',
    message: error instanceof Error ? error.message : String(error),
    details: null
  });
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toCommandError(error);
  }
}

// Account
export const appInfo = () => call<AppInfo>('app_info');
export const session = () => call<SessionState>('session');
export const suggestDeviceName = () => call<string>('suggest_device_name');
export const normalizeServerUrl = (raw: string) => call<string>('normalize_server_url', { raw });
export const startLogin = (serverUrl: string, deviceName: string) =>
  call<LoginStarted>('start_login', { serverUrl, deviceName });
export const reopenLoginBrowser = () => call<boolean>('reopen_login_browser');
export const cancelLogin = () => call<boolean>('cancel_login');
export const signOut = () => call<void>('sign_out');
export const serverVersion = () => call<string>('server_version');

// Linked folders
export const folders = () => call<LinkedFolder[]>('folders');
export const refreshStatus = (folder: number) => call<FolderStatus>('refresh_status', { folder });
export const refreshAll = () => call<void>('refresh_all');
export const syncFolder = (folder: number) => call<SyncReport>('sync_folder', { folder });
export const syncAll = () => call<void>('sync_all');
export const keepLocal = (folder: number) => call<SyncReport>('keep_local', { folder });
export const keepServer = (folder: number) => call<SyncReport>('keep_server', { folder });
export const decideLater = (folder: number) => call<LinkedFolder>('decide_later', { folder });
export const cancelOp = (folder: number) => call<boolean>('cancel_op', { folder });

// Linking
export const scopes = () => call<ScopeInfo[]>('scopes');
export const serverFolders = (scopeId: string) => call<FolderInfo[]>('server_folders', { scopeId });
export const createScope = (name: string) => call<ScopeInfo>('create_scope', { name });
export const pickFolder = (title: string, start?: string) =>
  call<string | null>('pick_folder', { title, start });
export const linkFolder = (folderId: string, localPath: string, ignore: string[]) =>
  call<LinkedFolder>('link_folder', { folderId, localPath, ignore });
export const createAndLink = (
  scopeId: string,
  name: string,
  emoji: string | null,
  localPath: string,
  ignore: string[]
) => call<LinkedFolder>('create_and_link', { scopeId, name, emoji, localPath, ignore });
export const unlink = (folder: number) => call<void>('unlink', { folder });
export const relocate = (folder: number, localPath: string) =>
  call<LinkedFolder>('relocate', { folder, localPath });
export const setIgnore = (folder: number, patterns: string[]) =>
  call<LinkedFolder>('set_ignore', { folder, patterns });
export const setPaused = (folder: number, paused: boolean) =>
  call<LinkedFolder>('set_paused', { folder, paused });
export const refreshNames = () => call<LinkedFolder[]>('refresh_names');

// Misc
export const activity = (folder: number | null, limit: number) =>
  call<ActivityEntry[]>('activity', { folder, limit });
export const openFolder = (folder: number) => call<void>('open_folder', { folder });
export const openHistory = (folder: number) => call<void>('open_history', { folder });
export const openLogDir = () => call<void>('open_log_dir');
/** Tells the backend the UI language (its tray menu and notifications follow it). */
export const setUiLanguage = (lang: string) => call<void>('set_ui_language', { lang });

// Automatic sync
export const appSettings = () => call<AppSettings>('app_settings');
export const setAppSettings = (settings: AppSettingsUpdate) =>
  call<AppSettings>('set_app_settings', { settings });
export const pauseFor = (minutes: number) => call<AppSettings>('pause_for', { minutes });
export const resume = () => call<AppSettings>('resume');
export const setAutoSync = (folder: number, settings: AutoSyncSettings) =>
  call<LinkedFolder>('set_auto_sync', { folder, settings });
export const runningProcesses = () => call<string[]>('running_processes');
export const autoStates = () => call<AutoFolderState[]>('auto_states');

// App updates and versions
export const updateState = () => call<UpdateState>('update_state');
export const checkUpdate = () => call<UpdateState>('check_update');
export const installUpdate = () => call<UpdateState>('install_update');
export const openReleasePage = () => call<void>('open_release_page');
export const versionInfo = () => call<VersionInfo>('version_info');
