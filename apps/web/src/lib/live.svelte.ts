// Live updates: one `EventSource` on `/api/v1/events` while signed in (client contract
// in STRUCTURE.md "Real-time events"). On every (re)connect the heads of all folders
// are fetched; with `realtime: local` (no Redis fan-out between server instances)
// they are polled every few minutes with the `ETag`. Listeners get the server events
// plus two synthetic ones: `folder_head` for a head found changed by polling, and
// `resync` after a reconnect (anything may have changed: reload what is shown).
import type { ServerEvent, SyncHead } from '@cloudit/api-types';
import { toast } from '@cloudit/ui';
import { t } from '@cloudit/ui/i18n';
import { eventsUrl, getFolder, getSyncHeads } from './api';
import { session } from './session.svelte';

export type LiveEvent = ServerEvent | { type: 'resync'; data: null };
type Listener = (event: LiveEvent) => void;

const POLL_MS = 5 * 60 * 1000;
const NAMES = [
  'hello',
  'folder_head',
  'conflict',
  'history_changed',
  'folder_changed',
  'scope_changed',
  'membership_changed',
  'usage_changed'
] as const;

// Not reactive on purpose: nothing renders from the set of listeners.
// eslint-disable-next-line svelte/prefer-svelte-reactivity
const listeners = new Set<Listener>();

/** Subscribes to live events; returns the unsubscribe function (use it in `$effect`). */
export function onLive(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function emit(event: LiveEvent): void {
  for (const listener of [...listeners]) {
    try {
      listener(event);
    } catch (err) {
      console.error('live listener failed', err);
    }
  }
}

class Live {
  /** `shared` when the server fans events out across instances, else `local`. */
  realtime = $state<'shared' | 'local' | null>(null);
  connected = $state(false);

  #source: EventSource | undefined;
  #poll: ReturnType<typeof setInterval> | undefined;
  #heads = new Map<string, SyncHead>();
  #etag: string | null = null;
  #seeded = false;

  start(): void {
    if (this.#source || typeof EventSource === 'undefined') return;
    const source = new EventSource(eventsUrl(), { withCredentials: true });
    this.#source = source;
    for (const name of NAMES) {
      source.addEventListener(name, (message) => this.#handle(name, message as MessageEvent));
    }
    source.onopen = () => {
      this.connected = true;
    };
    source.onerror = () => {
      this.connected = false;
      // CLOSED = the browser gave up (e.g. 401 after the session ended).
      if (source.readyState === EventSource.CLOSED) void this.#closed();
    };
  }

  stop(): void {
    this.#source?.close();
    this.#source = undefined;
    clearInterval(this.#poll);
    this.#poll = undefined;
    this.connected = false;
    this.realtime = null;
    this.#heads.clear();
    this.#etag = null;
    this.#seeded = false;
  }

  async #closed(): Promise<void> {
    this.stop();
    await session.load();
    if (session.signedIn) this.start();
  }

  #handle(name: (typeof NAMES)[number], message: MessageEvent): void {
    let data: unknown;
    try {
      data = JSON.parse(String(message.data));
    } catch {
      return;
    }
    const event = { type: name, data } as ServerEvent;
    if (event.type === 'hello') {
      this.realtime = event.data.realtime;
      clearInterval(this.#poll);
      this.#poll =
        event.data.realtime === 'local' ? setInterval(() => void this.#sync(), POLL_MS) : undefined;
      void this.#sync(true);
      return;
    }
    this.#track(event);
    if (event.type === 'conflict') void announceConflict(event.data.folder_id);
    emit(event);
  }

  /** Keeps the polling baseline in step with what the stream already reported. */
  #track(event: ServerEvent): void {
    if (event.type === 'folder_head') {
      const head = this.#heads.get(event.data.folder_id);
      if (head) head.head_snapshot_id = event.data.snapshot_id;
    } else if (event.type === 'conflict') {
      const head = this.#heads.get(event.data.folder_id);
      if (head) head.unresolved_conflicts += 1;
    }
  }

  /** Compares the folders' heads with the last known ones and reports differences. */
  async #sync(full = false): Promise<void> {
    let answer;
    try {
      answer = await getSyncHeads(full ? null : this.#etag);
    } catch {
      return; // offline or signed out: the stream's own error handling follows
    }
    if (!answer) return;
    this.#etag = answer.etag;
    const next = new Map(answer.heads.folders.map((head) => [head.folder_id, head]));
    if (this.#seeded) {
      // After a (re)connect anything may have changed; while polling, only heads did.
      if (full) emit({ type: 'resync', data: null });
      else this.#reportChanges(next);
    }
    this.#heads = next;
    this.#seeded = true;
  }

  #reportChanges(next: Map<string, SyncHead>): void {
    for (const [id, head] of next) {
      const old = this.#heads.get(id);
      if (!old) {
        emit({
          type: 'folder_changed',
          data: { scope_id: head.scope_id, folder_id: id, change: 'created' }
        });
      } else if (old.head_snapshot_id !== head.head_snapshot_id && head.head_snapshot_id) {
        emit({
          type: 'folder_head',
          data: { scope_id: head.scope_id, folder_id: id, snapshot_id: head.head_snapshot_id }
        });
      } else if (old.unresolved_conflicts !== head.unresolved_conflicts) {
        emit({ type: 'history_changed', data: { scope_id: head.scope_id, folder_id: id } });
      }
      if (old && head.unresolved_conflicts > old.unresolved_conflicts) {
        void announceConflict(id);
      }
    }
    for (const [id, head] of this.#heads) {
      if (!next.has(id)) {
        emit({
          type: 'folder_changed',
          data: { scope_id: head.scope_id, folder_id: id, change: 'deleted' }
        });
      }
    }
  }
}

async function announceConflict(folderId: string): Promise<void> {
  try {
    const folder = await getFolder(folderId);
    toast.info(t('live.conflict', { folder: folder.name }));
  } catch {
    toast.info(t('live.conflict_unknown'));
  }
}

export const live = new Live();
