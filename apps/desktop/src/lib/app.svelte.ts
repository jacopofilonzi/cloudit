// The app's shared state, fed by the backend's commands and events: who is signed in,
// the linked folders, their statuses, running operations and progress, the connection.
import type {
  Account,
  FolderOpEvent,
  FolderStatus,
  FolderStatusEvent,
  LinkedFolder,
  Connection,
  LoginEvent,
  OpAction,
  Progress,
  RemoteEvent
} from '@cloudit/api-types';
import * as api from './api';
import { CommandError } from './api';
import { needsSignIn } from './errors';
import { subscribe } from './events';
import { displayState, needsAttention } from './status';

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
        login: (e) => void this.#onLogin(e)
      });
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
    if (session.signed_in) {
      this.sessionEnded = false;
      await this.loadFolders();
      void api.refreshAll();
      void this.#loadServerVersion();
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

  async #loadServerVersion(): Promise<void> {
    try {
      this.serverVersion = await api.serverVersion();
    } catch {
      this.serverVersion = null;
    }
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
      if (error.kind !== 'cancelled') this.errors[folder] = error;
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
