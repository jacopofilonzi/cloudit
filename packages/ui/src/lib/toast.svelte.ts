// Transient notifications shown by `<Toaster />` (one per app, in the root layout).
// Messages are already-translated strings.

export type ToastKind = 'success' | 'info' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

/** Default display time; errors stay longer. */
export const TOAST_TIMEOUT_MS = 4000;
const ERROR_TIMEOUT_MS = 7000;

const items = $state<Toast[]>([]);
let nextId = 1;

/** The visible toasts, oldest first. */
export function getToasts(): readonly Toast[] {
  return items;
}

/** Hides a toast now. */
export function dismissToast(id: number): void {
  const index = items.findIndex((item) => item.id === id);
  if (index !== -1) items.splice(index, 1);
}

/** Shows a toast; `timeoutMs` 0 keeps it until dismissed. Returns its id. */
export function showToast(kind: ToastKind, message: string, timeoutMs?: number): number {
  const id = nextId++;
  items.push({ id, kind, message });
  const timeout = timeoutMs ?? (kind === 'error' ? ERROR_TIMEOUT_MS : TOAST_TIMEOUT_MS);
  if (timeout > 0) setTimeout(() => dismissToast(id), timeout);
  return id;
}

export const toast = {
  success: (message: string) => showToast('success', message),
  info: (message: string) => showToast('info', message),
  error: (message: string) => showToast('error', message)
};
