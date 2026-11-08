<script lang="ts">
  // Administration: admin-only (anyone else is sent home) with one route per tab.
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Tabs } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import { session } from '#lib/session.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  const admin = $derived(session.user?.is_admin ?? false);

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
  <Tabs value={current} {items} label={t('admin.title')} />
  {@render children()}
{/if}
