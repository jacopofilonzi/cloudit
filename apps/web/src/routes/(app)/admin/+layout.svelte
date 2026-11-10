<script lang="ts">
  // Administration: admin-only (anyone else is sent home) with one route per tab, and a
  // notice when a newer cloudit release than this server's is out (`update_check` job).
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Banner, Tabs } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import { session } from '#lib/session.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  const admin = $derived(session.user?.is_admin ?? false);
  const release = $derived(app.summary?.update_available ? app.summary.latest_release : null);

  $effect(() => {
    if (session.user && !admin) void goto(resolve('/(app)'), { replaceState: true });
  });

  type Tab = 'users' | 'invites' | 'providers' | 'settings' | 'maintenance';
  const current = $derived<Tab>(
    (['invites', 'providers', 'settings', 'maintenance'] as const).find((tab) =>
      page.url.pathname.includes(`/admin/${tab}`)
    ) ?? 'users'
  );
  const items = $derived([
    {
      value: 'users',
      label: t('admin.tab_users'),
      href: resolve('/(app)/admin/users'),
      badge: app.pendingUsers
    },
    { value: 'invites', label: t('admin.tab_invites'), href: resolve('/(app)/admin/invites') },
    {
      value: 'providers',
      label: t('admin.tab_providers'),
      href: resolve('/(app)/admin/providers')
    },
    { value: 'settings', label: t('admin.tab_settings'), href: resolve('/(app)/admin/settings') },
    {
      value: 'maintenance',
      label: t('admin.tab_maintenance'),
      href: resolve('/(app)/admin/maintenance')
    }
  ] satisfies { value: Tab; label: string; href: string; badge?: number }[]);
</script>

{#if admin}
  <div class="mb-3">
    <h1>{t('admin.title')}</h1>
    <p class="text-muted">{t('admin.subtitle')}</p>
  </div>
  {#if release && app.summary}
    <Banner variant="info" class="mb-3">
      {t('admin.update_available', {
        latest: release.version,
        current: app.summary.server_version
      })}
      <!-- eslint-disable svelte/no-navigation-without-resolve -- external release page -->
      <a
        class="font-medium text-primary-fg underline"
        href={release.url}
        target="_blank"
        rel="noopener noreferrer">{t('admin.update_link')}</a
      >
      <!-- eslint-enable svelte/no-navigation-without-resolve -->
    </Banner>
  {/if}
  <Tabs value={current} {items} label={t('admin.title')} />
  {@render children()}
{/if}
