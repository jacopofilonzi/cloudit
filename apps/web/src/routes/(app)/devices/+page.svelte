<script lang="ts">
  // Desktop devices (rename, revoke) and the web sessions of the account.
  import { onMount } from 'svelte';
  import Laptop from '@lucide/svelte/icons/laptop';
  import Pencil from '@lucide/svelte/icons/pencil';
  import type { DeviceInfo, SessionInfo } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    EmptyState,
    Skeleton,
    Table,
    TableActions,
    Tag,
    confirmAction,
    formatDate,
    formatDateTime,
    formatRelative,
    toast,
    type TagVariant
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import {
    deleteSession,
    listDevices,
    listSessions,
    revokeDevice,
    updateDevice
  } from '#lib/api.ts';
  import PromptModal from '#lib/components/PromptModal.svelte';
  import { errorText } from '#lib/errors.ts';
  import { browserLabel, osName } from '#lib/fields.ts';

  let devices = $state<DeviceInfo[] | null>(null);
  let sessions = $state<SessionInfo[] | null>(null);
  let error = $state<string | null>(null);
  let renaming = $state<DeviceInfo | null>(null);
  let renameOpen = $state(false);

  const lang = $derived(getLang());

  async function load() {
    try {
      [devices, sessions] = await Promise.all([listDevices(), listSessions()]);
      error = null;
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => void load());

  function status(device: DeviceInfo): { label: string; variant: TagVariant } {
    if (device.revoked_at) return { label: t('devices.status_revoked'), variant: 'danger' };
    if (device.expires_at && Date.parse(device.expires_at) < Date.now()) {
      return { label: t('devices.status_expired'), variant: 'neutral' };
    }
    return { label: t('devices.status_active'), variant: 'success' };
  }

  async function rename(name: string) {
    if (!renaming) return;
    await updateDevice(renaming.id, { name });
    toast.success(t('devices.renamed'));
    await load();
  }

  async function revoke(device: DeviceInfo) {
    const ok = await confirmAction({
      title: t('devices.revoke_title', { name: device.name }),
      message: t('devices.revoke_body'),
      confirmLabel: t('devices.revoke'),
      danger: true
    });
    if (!ok) return;
    try {
      await revokeDevice(device.id);
      toast.success(t('devices.revoked'));
    } catch (err) {
      toast.error(errorText(err));
    }
    await load();
  }

  async function signOutSession(item: SessionInfo) {
    try {
      await deleteSession(item.id);
      toast.success(t('sessions.signed_out'));
    } catch (err) {
      toast.error(errorText(err));
    }
    await load();
  }
</script>

<svelte:head><title>{t('devices.title')} · {t('app.name')}</title></svelte:head>

<div class="mb-[18px]">
  <h1>{t('devices.title')}</h1>
  <p class="text-muted">{t('devices.subtitle')}</p>
</div>

{#if error}
  <Banner variant="danger">{error}</Banner>
{:else if devices === null || sessions === null}
  <Skeleton class="h-40 w-full" />
{:else}
  {#if devices.length === 0}
    <EmptyState
      icon={Laptop}
      title={t('devices.empty_title')}
      description={t('devices.empty_body')}
    />
  {:else}
    <Table caption={t('devices.title')}>
      <thead>
        <tr>
          <th>{t('devices.col_name')}</th>
          <th>{t('devices.col_os')}</th>
          <th>{t('devices.col_seen')}</th>
          <th>{t('devices.col_expires')}</th>
          <th>{t('devices.col_status')}</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each devices as device (device.id)}
          {@const state = status(device)}
          <tr>
            <td class="font-semibold">{device.name}</td>
            <td>{osName(device.os)}</td>
            <td class="text-muted">
              {device.last_seen_at ? formatRelative(device.last_seen_at, lang) : t('common.never')}
            </td>
            <td class="text-muted">
              {device.expires_at ? formatDate(device.expires_at, lang) : t('devices.expires_never')}
            </td>
            <td><Tag variant={state.variant}>{state.label}</Tag></td>
            <td>
              <TableActions>
                {#if !device.revoked_at}
                  <Button
                    size="sm"
                    onclick={() => {
                      renaming = device;
                      renameOpen = true;
                    }}
                  >
                    <Pencil size={13} aria-hidden="true" />
                    {t('common.rename')}
                  </Button>
                  <Button size="sm" variant="danger" onclick={() => revoke(device)}>
                    {t('devices.revoke')}
                  </Button>
                {/if}
              </TableActions>
            </td>
          </tr>
        {/each}
      </tbody>
    </Table>
  {/if}

  <div class="mt-8 mb-3">
    <h2>{t('devices.sessions')}</h2>
    <p class="text-muted">{t('devices.sessions_desc')}</p>
  </div>
  <Table caption={t('devices.sessions')}>
    <thead>
      <tr>
        <th>{t('sessions.col_browser')}</th>
        <th>{t('sessions.col_active')}</th>
        <th>{t('sessions.col_expires')}</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each sessions as item (item.id)}
        <tr>
          <td>
            <span class="font-semibold" title={item.user_agent ?? undefined}>
              {browserLabel(item.user_agent) ?? t('sessions.unknown')}
            </span>
            {#if item.current}<Tag class="ms-2">{t('sessions.this')}</Tag>{/if}
          </td>
          <td class="text-muted">{formatRelative(item.last_seen_at, lang)}</td>
          <td class="text-muted">{formatDateTime(item.expires_at, lang)}</td>
          <td>
            <TableActions>
              {#if !item.current}
                <Button size="sm" variant="danger" onclick={() => signOutSession(item)}>
                  {t('sessions.sign_out')}
                </Button>
              {/if}
            </TableActions>
          </td>
        </tr>
      {/each}
    </tbody>
  </Table>
{/if}

<PromptModal
  bind:open={renameOpen}
  title={t('devices.rename_title')}
  label={t('field.name')}
  value={renaming?.name ?? ''}
  submitLabel={t('common.rename')}
  onsubmit={rename}
/>
