<script lang="ts">
  // The signed-in layout of the mockup: a 250px sidebar (brand, navigation, storage
  // meter, user) and the main column. The navigation is the caller's (`nav`
  // snippet: sections of `ShellNavItem`s); the storage meter and the user box are
  // built in.
  import type { Snippet } from 'svelte';
  import LogOut from '@lucide/svelte/icons/log-out';
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

  const percent = $derived(
    usage?.quota_bytes
      ? Math.min(100, Math.round((usage.used_bytes / usage.quota_bytes) * 100))
      : null
  );
</script>

<div class="grid min-h-screen md:grid-cols-[250px_1fr]">
  <aside
    class="flex flex-col gap-0.5 border-border bg-surface px-3.5 py-5 max-md:border-b md:sticky md:top-0 md:h-screen md:border-r"
  >
    <div class="flex items-center gap-2 px-2 pb-[18px] text-lg font-bold">
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

  <main class="min-w-0 px-6 py-7 md:px-9">
    {@render children()}
  </main>
</div>
