<script lang="ts">
  // The signed-in app inside the shell: the sidebar (scopes, account, administration),
  // the storage meter and the live connection that keeps all of it fresh.
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import Folder from '@lucide/svelte/icons/folder';
  import Laptop from '@lucide/svelte/icons/laptop';
  import Plus from '@lucide/svelte/icons/plus';
  import Shield from '@lucide/svelte/icons/shield';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import User from '@lucide/svelte/icons/user';
  import Users from '@lucide/svelte/icons/users';
  import { toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { logout } from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import AppShell from '#lib/components/AppShell.svelte';
  import ScopeCreateModal from '#lib/components/ScopeCreateModal.svelte';
  import ShellNavItem from '#lib/components/ShellNavItem.svelte';
  import ShellNavLabel from '#lib/components/ShellNavLabel.svelte';
  import { errorText } from '#lib/errors.ts';
  import { live, onLive } from '#lib/live.svelte.ts';
  import { session } from '#lib/session.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  const userId = $derived(session.user?.id);
  const admin = $derived(session.user?.is_admin ?? false);
  const here = $derived(page.url.pathname);
  let creating = $state(false);

  // Load everything once per signed-in user and open the live connection.
  $effect(() => {
    if (!userId) return;
    app.loadAll(admin).catch((err) => toast.error(errorText(err)));
    live.start();
    return () => live.stop();
  });

  // Administrators see new sign-ups waiting for approval (no event for them: poll).
  $effect(() => {
    if (!admin) return;
    const timer = setInterval(() => app.scheduleSummary(), 60_000);
    return () => clearInterval(timer);
  });

  $effect(() =>
    onLive((event) => {
      switch (event.type) {
        case 'scope_changed':
        case 'membership_changed':
        case 'folder_changed':
          app.scheduleScopes();
          app.scheduleUsage();
          break;
        case 'folder_head':
        case 'usage_changed':
          app.scheduleUsage();
          break;
        case 'resync':
          app.scheduleScopes();
          app.scheduleUsage();
          if (admin) app.scheduleSummary();
          break;
        default:
          break;
      }
    })
  );

  const isHere = (path: string) => here === path || here.startsWith(`${path}/`);

  async function signOut() {
    try {
      await logout();
    } catch (err) {
      toast.error(errorText(err));
      return;
    }
    live.stop();
    app.reset();
    // The root layout's guard then sends the browser to the login page.
    session.signedOut();
    // The login page shows the providers and sign-up mode as they are now.
    void session.refreshConfig().catch(() => undefined);
  }
</script>

{#if session.user}
  <AppShell user={session.user} usage={app.usage} onsignout={signOut}>
    {#snippet nav()}
      <ShellNavLabel>
        {t('shell.scopes')}
        {#snippet action()}
          <button
            type="button"
            class="btn btn-ghost btn-sm -my-1 px-1.5"
            aria-label={t('shell.new_scope')}
            title={t('shell.new_scope')}
            onclick={() => (creating = true)}
          >
            <Plus size={15} aria-hidden="true" />
          </button>
        {/snippet}
      </ShellNavLabel>
      {#if app.scopes?.length === 0}
        <p class="px-2.5 py-1 text-xs text-muted">{t('shell.no_scopes')}</p>
      {/if}
      {#each app.scopes ?? [] as scope (scope.id)}
        {@const href = resolve('/(app)/scopes/[id]', { id: scope.id })}
        <ShellNavItem {href} current={isHere(href)} count={scope.folder_count}>
          {#if scope.role === 'owner'}
            <Folder size={16} class="flex-none text-muted" aria-hidden="true" />
          {:else}
            <Users size={16} class="flex-none text-muted" aria-hidden="true" />
          {/if}
          <span class="truncate">{scope.name}</span>
          {#if scope.role !== 'owner'}
            <span class="truncate text-xs text-muted">
              · {t('shell.scope_of', { name: scope.owner.display_name })}
            </span>
          {/if}
        </ShellNavItem>
      {/each}

      <ShellNavLabel>{t('shell.account')}</ShellNavLabel>
      <ShellNavItem href={resolve('/(app)/devices')} current={isHere(resolve('/(app)/devices'))}>
        <Laptop size={16} class="flex-none text-muted" aria-hidden="true" />
        {t('shell.devices')}
      </ShellNavItem>
      <ShellNavItem href={resolve('/(app)/trash')} current={isHere(resolve('/(app)/trash'))}>
        <Trash2 size={16} class="flex-none text-muted" aria-hidden="true" />
        {t('shell.trash')}
      </ShellNavItem>
      <ShellNavItem href={resolve('/(app)/profile')} current={isHere(resolve('/(app)/profile'))}>
        <User size={16} class="flex-none text-muted" aria-hidden="true" />
        {t('shell.profile')}
      </ShellNavItem>

      {#if admin}
        <ShellNavLabel>{t('shell.server')}</ShellNavLabel>
        <ShellNavItem
          href={resolve('/(app)/admin')}
          current={isHere(resolve('/(app)/admin'))}
          badge={app.pendingUsers}
        >
          <Shield size={16} class="flex-none text-muted" aria-hidden="true" />
          {t('shell.admin')}
        </ShellNavItem>
      {/if}
    {/snippet}
    {@render children()}
  </AppShell>

  <ScopeCreateModal bind:open={creating} />
{/if}
