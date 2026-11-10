<script lang="ts">
  // The signed-in shell: sidebar (navigation with the attention badge, server state,
  // account), the app update and global pause banners, and the page.
  import Clock from '@lucide/svelte/icons/clock';
  import FolderSync from '@lucide/svelte/icons/folder-sync';
  import SettingsIcon from '@lucide/svelte/icons/settings';
  import type { Component, Snippet } from 'svelte';
  import { page } from '$app/state';
  import { resolve } from '$app/paths';
  import { Avatar, Banner, Button, Logo, StatusDot, formatTime } from '@cloudit/ui';
  import { getLang, t, type MessageKey } from '@cloudit/ui/i18n';
  import { reportError } from '#lib/actions.ts';
  import UpdateBanner from '#lib/components/UpdateBanner.svelte';
  import { app } from '#lib/app.svelte.ts';
  import { tn } from '#lib/plural.ts';

  let { children }: { children: Snippet } = $props();

  const items: {
    href: '/(app)/folders' | '/(app)/activity' | '/(app)/settings';
    label: MessageKey;
    icon: Component<{ size?: number }>;
  }[] = [
    { href: '/(app)/folders', label: 'desktop.nav.folders', icon: FolderSync },
    { href: '/(app)/activity', label: 'desktop.nav.activity', icon: Clock },
    { href: '/(app)/settings', label: 'desktop.nav.settings', icon: SettingsIcon }
  ];

  const host = $derived(app.account ? new URL(app.account.server_url).host : '');
  const dot = $derived(
    app.connection === 'online' ? 'ok' : app.connection === 'offline' ? 'warn' : 'off'
  );
  const connectionText = $derived(
    t(
      app.connection === 'online'
        ? 'desktop.shell.online'
        : app.connection === 'offline'
          ? 'desktop.shell.offline'
          : 'desktop.shell.connecting',
      { server: host }
    )
  );
  const current = (href: string) => page.url.pathname.startsWith(resolve(href as '/(app)/folders'));
</script>

<div class="flex h-screen">
  <aside
    class="flex w-[200px] flex-none flex-col gap-0.5 border-r border-border bg-surface px-2.5 py-3.5"
  >
    <div class="flex items-center gap-2 px-2 pb-3 text-[15px] font-bold">
      <Logo size={24} />
      {t('app.name')}
    </div>
    <nav class="flex flex-col gap-0.5">
      {#each items as item (item.href)}
        {@const Icon = item.icon}
        <a
          href={resolve(item.href)}
          class={[
            'flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] no-underline',
            current(item.href)
              ? 'bg-primary-soft font-semibold text-primary-fg'
              : 'text-text hover:bg-surface-alt'
          ]}
          aria-current={current(item.href) ? 'page' : undefined}
        >
          <Icon size={16} />
          {t(item.label)}
          {#if item.href === '/(app)/folders' && app.attention > 0}
            <span
              class="ms-auto rounded-full bg-warning px-1.5 text-[10px] text-on-primary"
              title={tn('desktop.shell.attention', app.attention)}
            >
              {app.attention}
            </span>
          {/if}
        </a>
      {/each}
    </nav>
    <div class="flex-1"></div>
    <div class="flex items-center gap-1.5 px-2 pb-1.5 text-xs text-muted" title={connectionText}>
      <StatusDot status={dot} />
      <span class="truncate">{host}</span>
    </div>
    {#if app.account}
      <div class="mt-1 flex items-center gap-2 border-t border-border px-1.5 pt-2.5">
        <Avatar name={app.account.display_name} key={app.account.user_id} size={26} />
        <div class="min-w-0">
          <b class="block truncate">{app.account.display_name}</b>
          <span class="block truncate text-xs text-muted">{app.account.device_name}</span>
        </div>
      </div>
    {/if}
  </aside>
  <main class="min-w-0 flex-1 overflow-auto px-5 py-[18px]">
    <UpdateBanner />
    {#if app.pausedUntil}
      <Banner variant="info" class="mb-3">
        {t('desktop.pause.banner', { time: formatTime(app.pausedUntil, getLang()) })}
        {#snippet actions()}
          <Button size="sm" onclick={() => app.resume().catch(reportError)}>
            {t('desktop.pause.resume')}
          </Button>
        {/snippet}
      </Banner>
    {/if}
    {@render children()}
  </main>
</div>
