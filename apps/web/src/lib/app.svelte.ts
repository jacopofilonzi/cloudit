// Data every signed-in page shares: the scopes of the sidebar, the storage meter and
// (for admins) the number of accounts waiting for approval and the release check. Pages call `load*` after
// their own changes; live events call the `schedule*` variants, which merge bursts.
import type { AdminSummary, ScopeInfo, UsageInfo } from '@cloudit/api-types';
import { getAdminSummary, getUsage, listScopes } from './api';
import { coalesced } from './coalesce';

class AppData {
  scopes = $state<ScopeInfo[] | null>(null);
  usage = $state<UsageInfo | null>(null);
  pendingUsers = $state(0);
  /** The last admin summary (server version, latest release). */
  summary = $state<AdminSummary | null>(null);

  async loadScopes(): Promise<void> {
    this.scopes = await listScopes();
  }

  async loadUsage(): Promise<void> {
    this.usage = await getUsage();
  }

  async loadSummary(): Promise<void> {
    const summary = await getAdminSummary();
    this.summary = summary;
    this.pendingUsers = summary.pending_users;
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
  }
}

export const app = new AppData();
