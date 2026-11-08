<script lang="ts">
  // The signed-in layout of the mockup: a 250px sidebar (brand, navigation, storage
  // meter, user) and the main column. The navigation is the caller's (`nav`
  // snippet: sections of `ShellNavItem`s); the storage meter and the user box are
  // built in.
  import type { Snippet } from 'svelte';
  import LogOut from '@lucide/svelte/icons/log-out';
  import MenuIcon from '@lucide/svelte/icons/menu';
  import X from '@lucide/svelte/icons/x';
  import { page } from '$app/state';
  import type { UsageInfo, UserProfile } from '@cloudit/api-types';
  import { Avatar, Logo, formatBytes } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';

  let {
    user,
    usage,
    onsignout,
    nav,
    children
  }: {
    user: UserProfile;
    usage: UsageInfo | null;
    onsignout: () => void;
    nav: Snippet;
    children: Snippet;
  } = $props();

  // Below `md` the sidebar is a drawer opened from the top bar; navigating closes it.
  let menuOpen = $state(false);
  $effect(() => {
    void page.url.pathname;
    menuOpen = false;
  });

  const percent = $derived(
    usage?.quota_bytes
      ? Math.min(100, Math.round((usage.used_bytes / usage.quota_bytes) * 100))
      : null
  );
</script>

<div class="min-h-screen md:grid md:grid-cols-[250px_1fr]">
  <header class="flex items-center gap-2 border-b border-border bg-surface px-4 py-2.5 md:hidden">
    <button
      type="button"
      class="btn btn-ghost btn-sm"
      aria-label={menuOpen ? t('shell.close_menu') : t('shell.open_menu')}
      aria-expanded={menuOpen}
      aria-controls="shell-sidebar"
      onclick={() => (menuOpen = !menuOpen)}
    >
      {#if menuOpen}<X size={18} aria-hidden="true" />{:else}<MenuIcon
          size={18}
          aria-hidden="true"
        />{/if}
    </button>
    <Logo />
    <b class="text-lg">{t('app.name')}</b>
  </header>

  <aside
    id="shell-sidebar"
    class={[
      'flex-col gap-0.5 border-border bg-surface px-3.5 py-5 max-md:border-b md:sticky md:top-0 md:flex md:h-screen md:border-r',
      menuOpen ? 'flex' : 'hidden'
    ]}
  >
    <div class="flex items-center gap-2 px-2 pb-[18px] text-lg font-bold max-md:hidden">
      <Logo />
      {t('app.name')}
    </div>

    <nav aria-label={t('shell.main_nav')} class="flex min-h-0 flex-col gap-0.5 overflow-y-auto">
      {@render nav()}
    </nav>

    <div class="flex-1"></div>

    {#if usage}
      <div class="mt-3 rounded-[10px] bg-surface-alt p-3 text-xs text-muted">
        {t('shell.storage')}
        {#if percent !== null}
          <div
            class="mt-2 mb-1 h-1.5 overflow-hidden rounded-full bg-border"
            role="meter"
            aria-label={t('shell.storage')}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          >
            <i class="block h-full bg-primary" style:width="{percent}%"></i>
          </div>
          {t('shell.storage_of', {
            used: formatBytes(usage.used_bytes, getLang()),
            quota: formatBytes(usage.quota_bytes ?? 0, getLang())
          })}
        {:else}
          <div class="mt-1">
            {t('shell.storage_unlimited', { used: formatBytes(usage.used_bytes, getLang()) })}
          </div>
        {/if}
      </div>
    {/if}

    <div class="flex items-center gap-2.5 px-2 pt-3">
      <Avatar name={user.display_name} key={user.id} />
      <div class="min-w-0 flex-1">
        <b class="block truncate">{user.display_name}</b>
        <span class="block truncate text-xs text-muted">{user.email}</span>
      </div>
      <button
        type="button"
        class="btn btn-ghost btn-sm"
        title={t('shell.sign_out')}
        aria-label={t('shell.sign_out')}
        onclick={onsignout}
      >
        <LogOut size={16} aria-hidden="true" />
      </button>
    </div>
  </aside>

  <main class="min-w-0 px-4 py-6 sm:px-6 md:px-9 md:py-7">
    {@render children()}
  </main>
</div>
