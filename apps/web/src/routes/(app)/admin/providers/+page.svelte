<script lang="ts">
  // Sign-in providers: order on the login page, test, edit, delete, add.
  import { onMount } from 'svelte';
  import { resolve } from '$app/paths';
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Plus from '@lucide/svelte/icons/plus';
  import type { ProviderInfo, ProviderPreset, ProviderTestResult } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Card,
    EmptyState,
    ProviderIcon,
    Skeleton,
    Tag,
    confirmAction,
    toast
  } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import {
    deleteProvider,
    getSettings,
    listProviderPresets,
    listProviders,
    testProvider,
    updateProvider
  } from '#lib/api.ts';
  import ProviderModal from '#lib/components/ProviderModal.svelte';
  import { errorText } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  let providers = $state<ProviderInfo[] | null>(null);
  let presets = $state<ProviderPreset[]>([]);
  let publicUrl = $state<string | null>(null);
  let error = $state<string | null>(null);
  let modalOpen = $state(false);
  let editing = $state<ProviderInfo | null>(null);
  let testing = $state<string | null>(null);
  let results = $state<Record<string, ProviderTestResult | { ok: false; code: null }>>({});
  let moving = $state(false);

  async function load() {
    try {
      providers = (await listProviders()).toSorted((a, b) => a.position - b.position);
      error = null;
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => {
    void load();
    listProviderPresets().then(
      (list) => (presets = list),
      (err) => (error = errorText(err))
    );
    getSettings().then(
      (s) => (publicUrl = s.effective_public_url),
      () => undefined
    );
  });

  // The login page reads the provider list from the session's config: keep it fresh.
  async function refreshConfig() {
    await session.refreshConfig().catch(() => undefined);
  }

  async function saved() {
    await load();
    await refreshConfig();
    toast.success(t('common.saved'));
  }

  function add() {
    editing = null;
    modalOpen = true;
  }

  function edit(provider: ProviderInfo) {
    editing = provider;
    modalOpen = true;
  }

  async function move(index: number, delta: -1 | 1) {
    if (!providers || moving) return;
    const order = [...providers];
    const [item] = order.splice(index, 1);
    if (!item) return;
    order.splice(index + delta, 0, item);
    moving = true;
    try {
      // Positions become the list indexes; only providers whose position changes are written.
      for (const [position, provider] of order.entries()) {
        if (provider.position !== position) await updateProvider(provider.id, { position });
      }
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      moving = false;
    }
    await load();
    await refreshConfig();
  }

  async function test(provider: ProviderInfo) {
    testing = provider.id;
    try {
      results[provider.id] = await testProvider(provider.id);
    } catch (err) {
      results[provider.id] = { ok: false, code: null };
      toast.error(errorText(err));
    } finally {
      testing = null;
    }
  }

  async function remove(provider: ProviderInfo) {
    const ok = await confirmAction({
      title: t('admin.provider_delete_title', { name: provider.display_name }),
      message: t('admin.provider_delete_body', { id: provider.id }),
      confirmLabel: t('common.delete'),
      danger: true
    });
    if (!ok) return;
    try {
      await deleteProvider(provider.id);
      toast.success(t('admin.provider_deleted'));
    } catch (err) {
      toast.error(errorText(err));
    }
    await load();
    await refreshConfig();
  }

  function groupsSummary(provider: ProviderInfo): string | null {
    const parts: string[] = [];
    if (provider.admin_groups.length > 0) {
      parts.push(t('admin.provider_admin_groups_n', { groups: provider.admin_groups.join(', ') }));
    }
    if (provider.allowed_groups.length > 0) {
      parts.push(
        t('admin.provider_allowed_groups_n', { groups: provider.allowed_groups.join(', ') })
      );
    }
    return parts.length > 0 ? parts.join(' · ') : null;
  }
</script>

<svelte:head>
  <title>{t('admin.tab_providers')} · {t('admin.title')} · {t('app.name')}</title>
</svelte:head>

<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
  <p class="max-w-[60ch] text-muted">{t('admin.providers_desc')}</p>
  <Button variant="primary" onclick={add} disabled={presets.length === 0}>
    <Plus size={15} aria-hidden="true" />{t('admin.provider_add')}
  </Button>
</div>

{#if providers !== null && !publicUrl}
  <Banner variant="warning" class="mb-4" title={t('admin.provider_no_url')}>
    {t('admin.provider_no_url_body')}
    {#snippet actions()}
      <Button size="sm" href={resolve('/(app)/admin/settings')}>{t('admin.tab_settings')}</Button>
    {/snippet}
  </Banner>
{/if}

{#if error}
  <Banner variant="danger">
    {error}
    {#snippet actions()}<Button size="sm" onclick={load}>{t('common.retry')}</Button>{/snippet}
  </Banner>
{:else if providers === null}
  <Skeleton class="h-40 w-full" />
{:else if providers.length === 0}
  <EmptyState
    icon={KeyRound}
    title={t('admin.providers_empty')}
    description={t('admin.providers_empty_body')}
  />
{:else}
  <ul class="flex flex-col gap-3" aria-label={t('admin.tab_providers')}>
    {#each providers as provider, index (provider.id)}
      {@const result = results[provider.id]}
      {@const groups = groupsSummary(provider)}
      <li data-provider={provider.id}>
        <Card class="flex flex-wrap items-center gap-4">
          <ProviderIcon icon={provider.icon} kind={provider.kind} size={36} />
          <div class="min-w-0 flex-1 basis-60">
            <div class="flex flex-wrap items-center gap-2">
              <h3 class="truncate">{provider.display_name}</h3>
              <span class="font-mono text-xs text-muted">{provider.id}</span>
              <Tag variant={provider.enabled ? 'success' : 'neutral'}>
                {provider.enabled ? t('admin.provider_on') : t('admin.provider_off')}
              </Tag>
              <Tag variant={provider.new_accounts === 'auto' ? 'primary' : 'warning'}>
                {provider.new_accounts === 'auto'
                  ? t('admin.new_accounts_auto_tag')
                  : t('admin.new_accounts_approval_tag')}
              </Tag>
            </div>
            <p class="truncate text-xs text-muted">
              {provider.issuer_url ?? provider.tenant ?? provider.kind}
            </p>
            {#if groups}<p class="text-xs text-muted">{groups}</p>{/if}
            {#if result}
              <p class="mt-1 text-xs" role="status">
                {#if result.ok}
                  <Tag variant="success">{t('admin.provider_test_ok')}</Tag>
                {:else}
                  <Tag variant="danger">
                    {result.code === 'provider_discovery_failed'
                      ? t('error.provider_discovery_failed')
                      : result.code === 'provider_error'
                        ? t('error.provider_error')
                        : t('admin.provider_test_failed')}
                  </Tag>
                {/if}
              </p>
            {/if}
          </div>
          <div class="flex flex-none flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              aria-label={t('admin.provider_move_up')}
              title={t('admin.provider_move_up')}
              disabled={index === 0 || moving}
              onclick={() => move(index, -1)}
            >
              <ArrowUp size={14} aria-hidden="true" />
            </Button>
            <Button
              size="sm"
              aria-label={t('admin.provider_move_down')}
              title={t('admin.provider_move_down')}
              disabled={index === providers.length - 1 || moving}
              onclick={() => move(index, 1)}
            >
              <ArrowDown size={14} aria-hidden="true" />
            </Button>
            <Button size="sm" loading={testing === provider.id} onclick={() => test(provider)}>
              {t('admin.provider_test')}
            </Button>
            <Button size="sm" onclick={() => edit(provider)}>
              <Pencil size={13} aria-hidden="true" />{t('admin.provider_edit')}
            </Button>
            <Button size="sm" variant="danger" onclick={() => remove(provider)}>
              {t('common.delete')}
            </Button>
          </div>
        </Card>
      </li>
    {/each}
  </ul>
{/if}

<ProviderModal bind:open={modalOpen} {presets} {editing} {publicUrl} onsaved={saved} />
