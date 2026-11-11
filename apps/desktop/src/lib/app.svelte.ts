// The app's shared state, fed by the backend's commands and events: who is signed in,
// the linked folders, their statuses, running operations and progress, the connection.
import type {
  AppSettings,
  AutoFolderState,
  Account,
  Compat,
  FolderOpEvent,
  FolderStatus,
  FolderStatusEvent,
  LinkedFolder,
  Connection,
  LoginEvent,
  NavigateEvent,
  OpAction,
  Progress,
  RemoteEvent,
  UpdateState,
  VersionInfo
} from '@cloudit/api-types';
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import * as api from './api';
import { CommandError } from './api';
import { needsSignIn } from './errors';
import { subscribe } from './events';
import { displayState, needsAttention } from './status';

/** Only unreadable (locked) files stopped the operation. */
function lockedOnly(error: CommandError): boolean {
  const details = error.details;
  return (
    details?.type === 'scan_problems' &&
    details.problems.length > 0 &&
    details.problems.every((p) => p.kind === 'unreadable')
  );
}

class AppStore {
  /** The first session / folder load is done. */
  ready = $state(false);
  startError = $state<CommandError | null>(null);
  account = $state<Account | null>(null);
  signedIn = $state(false);
  /** The token was refused or lost (shown on the welcome page). */
  sessionEnded = $state(false);
  tokenWarning = $state(false);
  appVersion = $state('');
  logDir = $state('');
  serverVersion = $state<string | null>(null);
  connection = $state<Connection>('connecting');

  folders = $state<LinkedFolder[]>([]);
  /** The folder selected in the list (the first one when unset). */
  selected = $state<number | null>(null);
  statuses = $state<Record<number, FolderStatus>>({});
  /** Operations running now, by linked folder. */
  busy = $state<Record<number, OpAction>>({});
  progress = $state<Record<number, Progress>>({});
  /** The last failed operation per folder, until the next one starts. */
  errors = $state<Record<number, CommandError>>({});
  /** The last browser sign-in outcome, for the welcome page. */
  login = $state<LoginEvent | null>(null);
  /** The device-wide automatic sync switch. */
  autoSync = $state(true);
  /** Desktop notifications for conflicts and errors. */
  notifications = $state(true);
  /** The app starts with the system. */
  autostart = $state(false);
  /** Automatic sync is paused until then (RFC 3339), or not paused. */
  pausedUntil = $state<string | null>(null);
  /** What automatic sync shows of each automatic folder (absent: not automatic now). */
  auto = $state<Record<number, AutoFolderState>>({});
  /** The app updater (checks run in the backend). */
  update = $state<UpdateState | null>(null);
  /** This app's, the server's and the latest release's versions. */
  versions = $state<VersionInfo | null>(null);
  /** Whether the server works with this app (sync stops when it doesn't). */
  compat = $state<Compat>({ status: 'ok' });

  #stop: (() => void) | null = null;

  /** Folders that need the user (conflict, changes, problems...). */
  get attention(): number {
    return this.folders.filter((f) => needsAttention(displayState(f, this.statuses[f.id]))).length;
  }

  /** Folders where this computer and the server both changed. */
  get conflicts(): LinkedFolder[] {
    return this.folders.filter((f) => displayState(f, this.statuses[f.id]) === 'conflict');
  }

  /** Loads the session and subscribes to the backend; call once at startup. */
  async init(): Promise<void> {
    try {
      this.#stop = await subscribe({
        progress: (p) => (this.progress[p.folder] = p),
        status: (e) => this.#onStatus(e),
        op: (e) => this.#onOp(e),
        remote: (e) => this.#onRemote(e),
        login: (e) => void this.#onLogin(e),
        auto: (states) => this.#onAuto(states),
        settings: (settings) => this.#onSettings(settings),
        navigate: (event) => void this.#onNavigate(event),
        update: (state) => this.#onUpdate(state),
        compat: (compat) => (this.compat = compat)
      });
      this.#onUpdate(await api.updateState());
      const info = await api.appInfo();
      this.appVersion = info.version;
      this.logDir = info.log_dir;
      await this.loadSession();
      this.startError = null;
    } catch (error) {
      this.startError = api.toCommandError(error);
    } finally {
      this.ready = true;
    }
  }

  async loadSession(): Promise<void> {
    const session = await api.session();
    this.account = session.account;
    this.signedIn = session.signed_in;
    this.tokenWarning = session.token_warning;
    this.connection = session.connection;
    this.compat = session.compat;
    if (session.signed_in) {
      this.sessionEnded = false;
      await this.loadFolders();
      void api.refreshAll();
      void this.#loadServerVersion();
      void this.loadVersions();
      void this.#loadAuto();
    } else if (session.account) {
      // Signed in before, but the token is gone: sign in again.
      this.sessionEnded = true;
    }
  }

  async loadFolders(): Promise<void> {
    try {
      this.folders = await api.folders();
    } catch (error) {
      this.noteError(error);
    }
  }

  async #loadAuto(): Promise<void> {
    try {
      const [settings, states] = await Promise.all([api.appSettings(), api.autoStates()]);
      this.#onSettings(settings);
      this.#onAuto(states);
    } catch (error) {
      this.noteError(error);
    }
  }

  /** Turns automatic sync on or off for this computer. */
  async setAutoSync(on: boolean): Promise<void> {
    this.#onSettings(await api.setAppSettings({ auto_sync: on }));
  }

  async setNotifications(on: boolean): Promise<void> {
    this.#onSettings(await api.setAppSettings({ notifications: on }));
  }

  async setAutostart(on: boolean): Promise<void> {
    this.#onSettings(await api.setAppSettings({ autostart: on }));
  }

  /** Pauses automatic sync (manual syncs still work). */
  async pause(minutes: number): Promise<void> {
    this.#onSettings(await api.pauseFor(minutes));
  }

  async resume(): Promise<void> {
    this.#onSettings(await api.resume());
  }

  #onSettings(settings: AppSettings): void {
    this.autoSync = settings.auto_sync;
    this.notifications = settings.notifications;
    this.autostart = settings.autostart;
    this.pausedUntil = settings.paused_until;
  }

  /** The tray asked for a folder's page. */
  async #onNavigate(event: NavigateEvent): Promise<void> {
    if (!this.signedIn) return;
    this.selected = event.folder;
    await (event.page === 'conflict'
      ? goto(resolve('/(app)/folders/[id]/conflict', { id: String(event.folder) }))
      : goto(resolve('/(app)/folders')));
  }

  /** Whether the folder is synced automatically now (its setting and the switch). */
  isAuto(link: LinkedFolder): boolean {
    return this.autoSync && link.auto_sync;
  }

  #onAuto(states: AutoFolderState[]): void {
    this.auto = Object.fromEntries(states.map((s) => [s.folder, s]));
  }

  async #loadServerVersion(): Promise<void> {
    try {
      this.serverVersion = await api.serverVersion();
    } catch {
      this.serverVersion = null;
    }
  }

  /** Reads the versions again (after a check, or when the settings open). */
  async loadVersions(): Promise<void> {
    try {
      this.versions = await api.versionInfo();
    } catch {
      this.versions = null;
    }
  }

  /** Starts the update the user asked for. */
  async installUpdate(): Promise<void> {
    this.#onUpdate(await api.installUpdate());
  }

  async checkUpdate(): Promise<void> {
    this.#onUpdate(await api.checkUpdate());
  }

  /** Asks the server again whether it works with this app. */
  async checkCompat(): Promise<Compat> {
    this.compat = await api.checkCompat();
    return this.compat;
  }

  #onUpdate(state: UpdateState): void {
    const checked = state.checked_at !== this.update?.checked_at;
    this.update = state;
    if (checked && this.signedIn) void this.loadVersions();
  }

  /** Report an error from a command: a refused token means "sign in again". */
  noteError(error: unknown): void {
    if (needsSignIn(error)) this.#endSession();
  }

  #endSession(): void {
    this.signedIn = false;
    this.sessionEnded = true;
  }

  /** Clears everything after an explicit sign-out. */
  signedOut(): void {
    this.signedIn = false;
    this.sessionEnded = false;
    this.account = null;
    this.folders = [];
    this.statuses = {};
    this.busy = {};
    this.progress = {};
    this.errors = {};
    this.serverVersion = null;
    this.versions = null;
    this.auto = {};
    this.compat = { status: 'ok' };
  }

  #onStatus(event: FolderStatusEvent): void {
    if (event.status) this.statuses[event.folder] = event.status;
  }

  #onOp(event: FolderOpEvent): void {
    const folder = event.folder;
    if (event.stage === 'started') {
      this.busy[folder] = event.action;
      delete this.errors[folder];
      return;
    }
    delete this.busy[folder];
    delete this.progress[folder];
    if (event.error) {
      const error = new CommandError(event.error);
      // Automatic runs stopped by files a running game keeps locked just try again.
      const quiet = error.kind === 'cancelled' || (event.auto && lockedOnly(error));
      if (!quiet) this.errors[folder] = error;
      this.noteError(error);
    }
    void this.loadFolders();
  }

  #onRemote(event: RemoteEvent): void {
    switch (event.type) {
      case 'connected':
        this.connection = 'online';
        break;
      case 'disconnected':
        this.connection = 'offline';
        break;
      case 'folders_changed':
        void api.refreshNames().then(
          (folders) => (this.folders = folders),
          (error: unknown) => this.noteError(error)
        );
        break;
      case 'signed_out':
        this.#endSession();
        break;
      default:
        break;
    }
  }

  async #onLogin(event: LoginEvent): Promise<void> {
    this.login = event;
    if (event.result === 'signed') {
      this.sessionEnded = false;
      try {
        await this.loadSession();
      } catch (error) {
        this.startError = api.toCommandError(error);
      }
    }
  }

  dispose(): void {
    this.#stop?.();
  }
}

export const app = new AppStore();
