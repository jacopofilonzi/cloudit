<script lang="ts">
  // The signed-in app inside the shell. Milestone 5a: the sidebar lists the scopes
  // (read-only) and the storage meter; navigation targets come with 5b.
  import { onMount } from 'svelte';
  import Folder from '@lucide/svelte/icons/folder';
  import type { ScopeInfo, UsageInfo } from '@cloudit/api-types';
  import { toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { getUsage, listScopes, logout } from '#lib/api.ts';
  import AppShell from '#lib/components/AppShell.svelte';
  import ShellNavItem from '#lib/components/ShellNavItem.svelte';
  import ShellNavLabel from '#lib/components/ShellNavLabel.svelte';
  import { errorText } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  let scopes = $state<ScopeInfo[] | null>(null);
  let usage = $state<UsageInfo | null>(null);

  onMount(async () => {
    try {
      [scopes, usage] = await Promise.all([listScopes(), getUsage()]);
    } catch (err) {
      toast.error(errorText(err));
    }
  });

  async function signOut() {
    try {
      await logout();
    } catch (err) {
      toast.error(errorText(err));
      return;
    }
    // The root layout's guard then sends the browser to the login page.
    session.signedOut();
  }
</script>

{#if session.user}
  <AppShell user={session.user} {usage} onsignout={signOut}>
    {#snippet nav()}
      <ShellNavLabel>{t('shell.scopes')}</ShellNavLabel>
      {#if scopes?.length === 0}
        <p class="px-2.5 py-1 text-xs text-muted">{t('shell.no_scopes')}</p>
      {/if}
      {#each scopes ?? [] as scope (scope.id)}
        <ShellNavItem count={scope.folder_count}>
          <Folder size={16} class="flex-none text-muted" aria-hidden="true" />
          <span class="truncate">{scope.name}</span>
          {#if scope.role !== 'owner'}
            <span class="truncate text-xs text-muted">
              · {t('shell.scope_of', { name: scope.owner.display_name })}
            </span>
          {/if}
        </ShellNavItem>
      {/each}
    {/snippet}
    {@render children()}
  </AppShell>
{/if}
