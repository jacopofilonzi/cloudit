<script lang="ts">
  // Invite links: create (optionally bound to an email), copy once, revoke.
  import { onMount } from 'svelte';
  import MailPlus from '@lucide/svelte/icons/mail-plus';
  import Plus from '@lucide/svelte/icons/plus';
  import type { AdminUser, InviteInfo, InviteStatus, TokenTtl } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    EmptyState,
    Skeleton,
    Table,
    TableActions,
    Tag,
    confirmAction,
    formatDateTime,
    toast,
    type TagVariant
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { createInvite, getSettings, listInvites, listUsers, revokeInvite } from '#lib/api.ts';
  import { absoluteUrl } from '#lib/admin.ts';
  import InviteCreateModal from '#lib/components/InviteCreateModal.svelte';
  import LinkModal from '#lib/components/LinkModal.svelte';
  import { errorText } from '#lib/errors.ts';

  let invites = $state<InviteInfo[] | null>(null);
  let names = $state<Record<string, string>>({});
  let publicUrl = $state<string | null>(null);
  let error = $state<string | null>(null);
  let createOpen = $state(false);
  let linkOpen = $state(false);
  let linkUrl = $state('');
  let linkExpires = $state('');

  const lang = $derived(getLang());

  async function load() {
    try {
      invites = await listInvites();
      error = null;
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => {
    void load();
    getSettings().then(
      (s) => (publicUrl = s.effective_public_url),
      () => undefined
    );
    // Creators are shown by name: the first page of accounts covers small servers;
    // anyone else falls back to "unknown".
    listUsers({ limit: 200 }).then(
      (list) =>
        (names = Object.fromEntries(list.users.map((u: AdminUser) => [u.id, u.display_name]))),
      () => undefined
    );
  });

  const STATUS: Record<InviteStatus, { key: Parameters<typeof t>[0]; variant: TagVariant }> = {
    pending: { key: 'admin.invite_active', variant: 'success' },
    used: { key: 'admin.invite_used', variant: 'primary' },
    expired: { key: 'admin.invite_expired', variant: 'neutral' },
    revoked: { key: 'admin.invite_revoked', variant: 'danger' }
  };

  async function create(email: string | undefined, ttl: TokenTtl) {
    const link = await createInvite({ email, expires_in: ttl });
    linkUrl = absoluteUrl(link.path, publicUrl, location.origin);
    linkExpires = link.expires_at;
    linkOpen = true;
    await load();
  }

  async function revoke(invite: InviteInfo) {
    const ok = await confirmAction({
      title: t('admin.invite_revoke_title'),
      message: t('admin.invite_revoke_body'),
      confirmLabel: t('admin.invite_revoke'),
      danger: true
    });
    if (!ok) return;
    try {
      await revokeInvite(invite.id);
      toast.success(t('admin.invite_revoked_done'));
    } catch (err) {
      toast.error(errorText(err));
    }
    await load();
  }
</script>

<svelte:head>
  <title>{t('admin.tab_invites')} · {t('admin.title')} · {t('app.name')}</title>
</svelte:head>

<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
  <p class="max-w-[60ch] text-muted">{t('admin.invites_desc')}</p>
  <Button variant="primary" onclick={() => (createOpen = true)}>
    <Plus size={15} aria-hidden="true" />{t('admin.invite_new')}
  </Button>
</div>

{#if error}
  <Banner variant="danger">
    {error}
    {#snippet actions()}<Button size="sm" onclick={load}>{t('common.retry')}</Button>{/snippet}
  </Banner>
{:else if invites === null}
  <Skeleton class="h-40 w-full" />
{:else if invites.length === 0}
  <EmptyState
    icon={MailPlus}
    title={t('admin.invites_empty')}
    description={t('admin.invites_empty_body')}
  />
{:else}
  <Table caption={t('admin.tab_invites')}>
    <thead>
      <tr>
        <th>{t('admin.col_invite_for')}</th>
        <th>{t('admin.col_created_by')}</th>
        <th>{t('admin.col_expires')}</th>
        <th>{t('admin.col_status')}</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each invites as invite (invite.id)}
        {@const status = STATUS[invite.status]}
        <tr>
          <td>
            {#if invite.email}
              <span class="font-semibold">{invite.email}</span>
            {:else}
              <span class="text-muted">{t('admin.invite_anyone')}</span>
            {/if}
          </td>
          <td class="text-muted">
            {names[invite.created_by] ?? t('common.unknown')}
            <div class="text-xs">{formatDateTime(invite.created_at, lang)}</div>
          </td>
          <td class="text-muted">{formatDateTime(invite.expires_at, lang)}</td>
          <td>
            <Tag variant={status.variant}>{t(status.key)}</Tag>
            {#if invite.status === 'used' && invite.used_by}
              <div class="text-xs text-muted">{names[invite.used_by] ?? ''}</div>
            {/if}
          </td>
          <td>
            <TableActions>
              {#if invite.status === 'pending'}
                <Button size="sm" variant="danger" onclick={() => revoke(invite)}>
                  {t('admin.invite_revoke')}
                </Button>
              {/if}
            </TableActions>
          </td>
        </tr>
      {/each}
    </tbody>
  </Table>
{/if}

<InviteCreateModal bind:open={createOpen} oncreate={create} />
<LinkModal
  bind:open={linkOpen}
  title={t('admin.invite_link_title')}
  description={t('admin.invite_link_desc')}
  url={linkUrl}
  expiresAt={linkExpires}
/>
