// Promise-based confirmation for destructive actions, rendered by `<ConfirmHost />`
// (one per app, in the root layout). Never use the browser's `confirm()`.
//
//   if (await confirmAction({ title, message, confirmLabel, danger: true })) { … }

export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  /** Red confirm button. */
  danger?: boolean;
}

interface Pending extends ConfirmOptions {
  resolve: (confirmed: boolean) => void;
}

let pending = $state<Pending | null>(null);

/** The request waiting for an answer, if any. */
export function getPendingConfirm(): ConfirmOptions | null {
  return pending;
}

/** Asks the user; resolves `true` when confirmed. A new request cancels the previous one. */
export function confirmAction(options: ConfirmOptions): Promise<boolean> {
  pending?.resolve(false);
  return new Promise((resolve) => {
    pending = { ...options, resolve };
  });
}

/** Answers the pending request (called by `ConfirmHost`). */
export function answerConfirm(confirmed: boolean): void {
  const current = pending;
  pending = null;
  current?.resolve(confirmed);
}
