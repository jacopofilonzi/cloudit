// Data every signed-in page shares: the scopes of the sidebar, the storage meter and
// (for admins) the number of accounts waiting for approval and the release check. Pages call `load*` after
// their own changes; live events call the `schedule*` variants, which merge bursts.
import type { AdminSummary, ScopeInfo, UsageInfo } from '@cloudit/api-types';
import { checkForUpdates, getAdminSummary, getUsage, listScopes } from './api';
import { coalesced } from './coalesce';
import { checkResult, waitForCheck, type CheckResult } from './update-check.ts';

/** While the summary says a check is pending, it is read again this often, this many times. */
const PENDING_REFRESH_MS = 3000;
const PENDING_REFRESHES = 20;

class AppData {
  scopes = $state<ScopeInfo[] | null>(null);
  usage = $state<UsageInfo | null>(null);
  pendingUsers = $state(0);
  /** The last admin summary (server version, latest release, update check). */
  summary = $state<AdminSummary | null>(null);
  /** "Check now" is running (shared by every button that starts it). */
  checkingUpdates = $state(false);

  #pendingTimer: ReturnType<typeof setTimeout> | undefined;
  #pendingRefreshes = 0;

  async loadScopes(): Promise<void> {
    this.scopes = await listScopes();
  }

  async loadUsage(): Promise<void> {
    this.usage = await getUsage();
  }

  async loadSummary(): Promise<void> {
    this.#showSummary(await getAdminSummary());
    // Reading the summary may have started an update check on the server (when the
    // last one is older than 10 minutes): read it again shortly to show the result.
    clearTimeout(this.#pendingTimer);
    this.#pendingTimer = undefined;
    if (!this.summary?.update_check.pending || this.checkingUpdates) {
      this.#pendingRefreshes = 0;
    } else if (this.#pendingRefreshes < PENDING_REFRESHES) {
      this.#pendingRefreshes++;
      this.#pendingTimer = setTimeout(() => this.scheduleSummary(), PENDING_REFRESH_MS);
    }
  }

  #showSummary(summary: AdminSummary): void {
    this.summary = summary;
    this.pendingUsers = summary.pending_users;
  }

  /** "Check now": asks the server for an update check and waits for its result. */
  async checkUpdates(): Promise<CheckResult> {
    this.checkingUpdates = true;
    try {
      const last = await waitForCheck(await checkForUpdates(), getAdminSummary, (summary) =>
        this.#showSummary(summary)
      );
      return checkResult(last);
    } finally {
      this.checkingUpdates = false;
    }
  }

  /** The initial load; `admin` also reads the approval counter. */
  async loadAll(admin: boolean): Promise<void> {
    await Promise.all([this.loadScopes(), this.loadUsage(), admin ? this.loadSummary() : null]);
  }

  scheduleScopes = coalesced(() => this.loadScopes());
  scheduleUsage = coalesced(() => this.loadUsage(), 2000);
  scheduleSummary = coalesced(() => this.loadSummary());

  scope(id: string | undefined): ScopeInfo | undefined {
    return this.scopes?.find((scope) => scope.id === id);
  }

  /** After sign-out: nothing of this user may linger. */
  reset(): void {
    this.scopes = null;
    this.usage = null;
    this.pendingUsers = 0;
    this.summary = null;
    clearTimeout(this.#pendingTimer);
    this.#pendingTimer = undefined;
    this.#pendingRefreshes = 0;
  }
}

export const app = new AppData();
