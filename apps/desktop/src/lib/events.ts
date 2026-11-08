// Events pushed by the Rust backend (names mirror `src-tauri/src/events.rs`).
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  FolderOpEvent,
  FolderStatusEvent,
  LoginEvent,
  Progress,
  RemoteEvent
} from '@cloudit/api-types';

export const EVENTS = {
  progress: 'folder-progress',
  status: 'folder-status',
  op: 'folder-op',
  remote: 'remote',
  login: 'login-finished'
} as const;

export interface Handlers {
  progress: (event: Progress) => void;
  status: (event: FolderStatusEvent) => void;
  op: (event: FolderOpEvent) => void;
  remote: (event: RemoteEvent) => void;
  login: (event: LoginEvent) => void;
}

/** Subscribes to every backend event; the returned function unsubscribes all. */
export async function subscribe(handlers: Handlers): Promise<() => void> {
  const stops: UnlistenFn[] = await Promise.all([
    listen<Progress>(EVENTS.progress, (e) => handlers.progress(e.payload)),
    listen<FolderStatusEvent>(EVENTS.status, (e) => handlers.status(e.payload)),
    listen<FolderOpEvent>(EVENTS.op, (e) => handlers.op(e.payload)),
    listen<RemoteEvent>(EVENTS.remote, (e) => handlers.remote(e.payload)),
    listen<LoginEvent>(EVENTS.login, (e) => handlers.login(e.payload))
  ]);
  return () => stops.forEach((stop) => stop());
}
