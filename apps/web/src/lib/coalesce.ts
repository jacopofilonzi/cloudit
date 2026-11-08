// Merges bursts of refresh requests (an event storm after a big sync) into few runs:
// calls within `delayMs` share one run, and a call during a run schedules exactly
// one more after it, so the data read last is never older than the last call.

export function coalesced(run: () => Promise<void>, delayMs = 200): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let running = false;
  let again = false;

  async function fire(): Promise<void> {
    timer = undefined;
    running = true;
    try {
      await run();
    } catch {
      // The caller reports failures; a refresh must never throw into an event handler.
    } finally {
      running = false;
    }
    if (again) {
      again = false;
      schedule();
    }
  }

  function schedule(): void {
    if (running) {
      again = true;
      return;
    }
    if (timer === undefined) timer = setTimeout(() => void fire(), delayMs);
  }

  return schedule;
}
