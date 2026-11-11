// Events pushed by the Rust backend (names mirror `src-tauri/src/events.rs`).
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppSettings,
  AutoFolderState,
  Compat,
  FolderOpEvent,
  FolderStatusEvent,
  LoginEvent,
  NavigateEvent,
  Progress,
  RemoteEvent,
  UpdateState
} from '@cloudit/api-types';

export const EVENTS = {
  progress: 'folder-progress',
  status: 'folder-status',
  op: 'folder-op',
  remote: 'remote',
  login: 'login-finished',
  auto: 'auto-state',
  settings: 'app-settings',
  navigate: 'navigate',
  update: 'update-state',
  compat: 'compat'
} as const;

export interface Handlers {
  progress: (event: Progress) => void;
  status: (event: FolderStatusEvent) => void;
  op: (event: FolderOpEvent) => void;
  remote: (event: RemoteEvent) => void;
  login: (event: LoginEvent) => void;
  auto: (states: AutoFolderState[]) => void;
  settings: (settings: AppSettings) => void;
  navigate: (event: NavigateEvent) => void;
  update: (state: UpdateState) => void;
  compat: (compat: Compat) => void;
}

/** Subscribes to every backend event; the returned function unsubscribes all. */
export async function subscribe(handlers: Handlers): Promise<() => void> {
  const stops: UnlistenFn[] = await Promise.all([
    listen<Progress>(EVENTS.progress, (e) => handlers.progress(e.payload)),
    listen<FolderStatusEvent>(EVENTS.status, (e) => handlers.status(e.payload)),
    listen<FolderOpEvent>(EVENTS.op, (e) => handlers.op(e.payload)),
    listen<RemoteEvent>(EVENTS.remote, (e) => handlers.remote(e.payload)),
    listen<LoginEvent>(EVENTS.login, (e) => handlers.login(e.payload)),
    listen<AutoFolderState[]>(EVENTS.auto, (e) => handlers.auto(e.payload)),
    listen<AppSettings>(EVENTS.settings, (e) => handlers.settings(e.payload)),
    listen<NavigateEvent>(EVENTS.navigate, (e) => handlers.navigate(e.payload)),
    listen<UpdateState>(EVENTS.update, (e) => handlers.update(e.payload)),
    listen<Compat>(EVENTS.compat, (e) => handlers.compat(e.payload))
  ]);
  return () => stops.forEach((stop) => stop());
}
