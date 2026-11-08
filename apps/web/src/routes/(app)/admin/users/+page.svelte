<script lang="ts">
  // Accounts: approvals, roles, quotas, bans and password reset links.
  import { onMount } from 'svelte';
  import ChevronLeft from '@lucide/svelte/icons/chevron-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import Mail from '@lucide/svelte/icons/mail';
  import Users from '@lucide/svelte/icons/users';
  import type { AdminUser, ApprovalStatus, ProviderInfo } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    EmptyState,
    Menu,
    ProviderIcon,
    Segmented,
    Skeleton,
    Table,
    TableActions,
    Tag,
    confirmAction,
    formatBytes,
    formatDateTime,
    formatNumber,
    formatRelative,
    toast
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import {
    approveUser,
    createPasswordReset,
    getSettings,
    listProviders,
    listUsers,
    rejectUser,
    updateUser
  } from '#lib/api.ts';
  import { absoluteUrl } from '#lib/admin.ts';
  import { app } from '#lib/app.svelte.ts';
  import LinkModal from '#lib/components/LinkModal.svelte';
  import QuotaModal from '#lib/components/QuotaModal.svelte';
  import { errorText } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const PAGE = 25;
  type Filter = 'all' | 'pending' | 'rejected';

  let filter = $state<Filter>('all');
  let offset = $state(0);
  let users = $state<AdminUser[] | null>(null);
  let total = $state(0);
  let providers = $state<ProviderInfo[]>([]);
  let publicUrl = $state<string | null>(null);
  let defaultQuota = $state<number | null>(null);
  let error = $state<string | null>(null);

  let quotaUser = $state<AdminUser | null>(null);
  let quotaOpen = $state(false);
  let resetOpen = $state(false);
  let resetUrl = $state('');
  let resetExpires = $state('');

  const lang = $derived(getLang());
  const me = $derived(session.user?.id);

  async function load() {
    try {
      const status: ApprovalStatus | undefined = filter === 'all' ? undefined : filter;
      const answer = await listUsers({ limit: PAGE, offset, approval_status: status });
      // A page past the end (its last row was just handled): go back one page.
      if (answer.users.length === 0 && offset > 0) {
        offset = Math.max(0, offset - PAGE);
        return;
      }
      users = answer.users;
      total = answer.total;
      error = null;
    } catch (err) {
      error = errorText(err);
    }
  }

  onMount(() => {
    // Providers (icons) and settings (links, default quota) only decorate: ignore failures.
    listProviders().then(
      (list) => (providers = list),
      () => undefined
    );
    getSettings().then(
      (s) => {
        publicUrl = s.effective_public_url;
        defaultQuota = s.default_quota_bytes;
      },
      () => undefined
    );
    void app.loadSummary().catch(() => undefined);
  });

  // Reload when the filter or the page changes (the effect reads both).
  $effect(() => {
    void filter;
    void offset;
    void load();
  });

  function setFilter(next: Filter) {
    filter = next;
    offset = 0;
  }

  const provider = (id: string) => providers.find((p) => p.id === id);

  async function run(action: () => Promise<unknown>, done: string) {
    try {
      await action();
      toast.success(done);
    } catch (err) {
      toast.error(errorText(err));
    }
    await Promise.all([load(), app.loadSummary().catch(() => undefined)]);
  }

  const approve = (user: AdminUser) =>
    run(() => approveUser(user.id), t('admin.approved', { name: user.display_name }));

  async function reject(user: AdminUser) {
    const ok = await confirmAction({
      title: t('admin.reject_title', { name: user.display_name }),
      message: t('admin.reject_body'),
      confirmLabel: t('admin.reject'),
      danger: true
    });
    if (ok) await run(() => rejectUser(user.id), t('admin.rejected', { name: user.display_name }));
  }

  async function unban(user: AdminUser) {
    const ok = await confirmAction({
      title: t('admin.unban_title', { name: user.display_name }),
      message: t('admin.unban_body'),
      confirmLabel: t('admin.unban')
    });
    if (ok) await run(() => approveUser(user.id), t('admin.approved', { name: user.display_name }));
  }

  const toggleAdmin = (user: AdminUser) =>
    run(
      () => updateUser(user.id, { admin: !user.admin }),
      user.admin ? t('admin.admin_removed') : t('admin.admin_granted')
    );

  async function toggleDisabled(user: AdminUser) {
    if (!user.disabled_at) {
      const ok = await confirmAction({
        title: t('admin.disable_title', { name: user.display_name }),
        message: t('admin.disable_body'),
        confirmLabel: t('admin.disable'),
        danger: true
      });
      if (!ok) return;
    }
    await run(
      () => updateUser(user.id, { disabled: !user.disabled_at }),
      user.disabled_at ? t('admin.enabled') : t('admin.disabled')
    );
  }

  async function saveQuota(quota: number | null) {
    if (!quotaUser) return;
    await updateUser(quotaUser.id, { quota_bytes: quota });
    toast.success(t('admin.quota_saved'));
    await load();
  }

  async function reset(user: AdminUser) {
    try {
      const link = await createPasswordReset(user.id);
      resetUrl = absoluteUrl(link.path, publicUrl, location.origin);
      resetExpires = link.expires_at;
      resetOpen = true;
    } catch (err) {
      toast.error(errorText(err));
    }
  }

  function menu(user: AdminUser) {
    const items: { label: string; onclick: () => void; danger?: boolean }[] = [];
    if (user.id !== me) {
      items.push({
        label: user.admin ? t('admin.remove_admin') : t('admin.make_admin'),
        onclick: () => toggleAdmin(user)
      });
    }
    items.push(
      {
        label: t('admin.set_quota'),
        onclick: () => {
          quotaUser = user;
          quotaOpen = true;
        }
      },
      { label: t('admin.reset_password'), onclick: () => reset(user) }
    );
    if (user.id !== me) {
      items.push({
        label: user.disabled_at ? t('admin.enable') : t('admin.disable'),
        onclick: () => toggleDisabled(user),
        danger: !user.disabled_at
      });
    }
    return items;
  }

  const pending = $derived(filter === 'pending');
  const from = $derived(total === 0 ? 0 : offset + 1);
  const to = $derived(Math.min(offset + PAGE, total));
</script>

<svelte:head
  ><title>{t('admin.tab_users')} · {t('admin.title')} · {t('app.name')}</title></svelte:head
>

<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
  <Segmented
    name="user-filter"
    bind:value={() => filter, setFilter}
    options={[
      { value: 'all', label: t('admin.filter_all') },
      {
        value: 'pending',
        label:
          app.pendingUsers > 0
            ? t('admin.filter_pending_n', { n: app.pendingUsers })
            : t('admin.filter_pending')
      },
      { value: 'rejected', label: t('admin.filter_rejected') }
    ]}
  />
</div>

{#if filter === 'all' && app.pendingUsers > 0}
  <Banner variant="warning" class="mb-4" title={t('admin.pending_banner', { n: app.pendingUsers })}>
    {#snippet actions()}
      <Button size="sm" onclick={() => setFilter('pending')}>{t('admin.review')}</Button>
    {/snippet}
  </Banner>
{/if}

{#if error}
  <Banner variant="danger">
    {error}
    {#snippet actions()}<Button size="sm" onclick={load}>{t('common.retry')}</Button>{/snippet}
  </Banner>
{:else if users === null}
  <Skeleton class="h-48 w-full" />
{:else if users.length === 0}
  <EmptyState
    icon={Users}
    title={pending ? t('admin.empty_pending') : t('admin.empty_users')}
    description={pending ? t('admin.empty_pending_body') : undefined}
  />
{:else}
  <Table caption={t('admin.tab_users')}>
    <thead>
      <tr>
        <th>{t('admin.col_user')}</th>
        {#if pending}
          <th>{t('admin.col_source')}</th>
          <th>{t('admin.col_requested')}</th>
        {:else}
          <th>{t('admin.col_role')}</th>
          <th>{t('admin.col_status')}</th>
          <th>{t('admin.col_storage')}</th>
          <th>{t('admin.col_signin')}</th>
        {/if}
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each users as user (user.id)}
        <tr>
          <td>
            <span class="font-semibold">{user.display_name}</span>
            {#if user.id === me}<Tag class="ms-2" variant="neutral">{t('common.you')}</Tag>{/if}
            <div class="text-xs text-muted">{user.email}</div>
          </td>
          {#if pending}
            <td>
              {#if user.signup_source === 'password'}
                <span class="inline-flex items-center gap-1.5">
                  <KeyRound size={15} class="text-muted" aria-hidden="true" />
                  {t('admin.source_password')}
                </span>
              {:else if user.signup_source === 'invite'}
                <span class="inline-flex items-center gap-1.5">
                  <Mail size={15} class="text-muted" aria-hidden="true" />
                  {t('admin.source_invite')}
                </span>
              {:else if user.signup_source}
                {@const p = provider(user.signup_source)}
                <span class="inline-flex items-center gap-1.5">
                  <ProviderIcon icon={p?.icon ?? 'generic'} kind={p?.kind} size={18} />
                  {p?.display_name ?? user.signup_source}
                </span>
              {:else}
                <span class="text-muted">—</span>
              {/if}
            </td>
            <td class="text-muted" title={formatDateTime(user.created_at, lang)}>
              {formatRelative(user.created_at, lang)}
            </td>
          {:else}
            <td>
              {#if user.admin}
                <Tag>{t('admin.role_admin')}</Tag>
              {:else if user.admin_from_groups}
                <Tag variant="primary">{t('admin.role_admin_groups')}</Tag>
              {:else}
                <span class="text-muted">{t('admin.role_user')}</span>
              {/if}
            </td>
            <td>
              {#if user.approval_status === 'rejected'}
                <Tag variant="danger">{t('admin.status_rejected')}</Tag>
              {:else if user.approval_status === 'pending'}
                <Tag variant="warning">{t('admin.status_pending')}</Tag>
              {:else if user.disabled_at}
                <Tag variant="neutral">{t('admin.status_disabled')}</Tag>
              {:else}
                <Tag variant="success">{t('admin.status_active')}</Tag>
              {/if}
            </td>
            <td class="text-xs">
              <span class="font-semibold">{formatBytes(user.used_bytes, lang)}</span>
              <span class="text-muted">
                /
                {user.effective_quota_bytes === null
                  ? t('admin.unlimited')
                  : formatBytes(user.effective_quota_bytes, lang)}
              </span>
              {#if user.quota_bytes !== null}
                <Tag class="ms-1" variant="neutral">{t('admin.quota_own')}</Tag>
              {/if}
            </td>
            <td>
              <span class="inline-flex items-center gap-1">
                {#if user.has_password}
                  <span title={t('admin.signin_password')}>
                    <KeyRound
                      size={16}
                      class="text-muted"
                      aria-label={t('admin.signin_password')}
                    />
                  </span>
                {/if}
                {#each user.identity_providers as id (id)}
                  {@const p = provider(id)}
                  <span title={p?.display_name ?? id}>
                    <ProviderIcon icon={p?.icon ?? 'generic'} kind={p?.kind} size={18} />
                  </span>
                {/each}
              </span>
            </td>
          {/if}
          <td>
            <TableActions>
              {#if user.approval_status === 'pending'}
                <Button size="sm" variant="primary" onclick={() => approve(user)}>
                  {t('admin.approve')}
                </Button>
                <Button size="sm" variant="danger" onclick={() => reject(user)}>
                  {t('admin.reject')}
                </Button>
              {:else if user.approval_status === 'rejected'}
                <Button size="sm" onclick={() => unban(user)}>{t('admin.unban')}</Button>
              {:else}
                {#if user.id !== me}
                  <Button size="sm" variant="danger" onclick={() => reject(user)}>
                    {t('admin.reject')}
                  </Button>
                {/if}
                <Menu items={menu(user)} label={t('common.more_actions')} />
              {/if}
            </TableActions>
          </td>
        </tr>
      {/each}
    </tbody>
  </Table>

  {#if total > PAGE}
    <nav class="mt-3 flex items-center justify-end gap-3" aria-label={t('admin.pagination')}>
      <span class="text-xs text-muted">
        {t('admin.page_range', {
          from: formatNumber(from, lang),
          to: formatNumber(to, lang),
          total: formatNumber(total, lang)
        })}
      </span>
      <Button
        size="sm"
        disabled={offset === 0}
        onclick={() => (offset = Math.max(0, offset - PAGE))}
      >
        <ChevronLeft size={14} aria-hidden="true" />{t('admin.prev')}
      </Button>
      <Button size="sm" disabled={offset + PAGE >= total} onclick={() => (offset += PAGE)}>
        {t('admin.next')}<ChevronRight size={14} aria-hidden="true" />
      </Button>
    </nav>
  {/if}
{/if}

<QuotaModal bind:open={quotaOpen} user={quotaUser} {defaultQuota} onsave={saveQuota} />
<LinkModal
  bind:open={resetOpen}
  title={t('admin.reset_title')}
  description={t('admin.reset_desc')}
  url={resetUrl}
  expiresAt={resetExpires}
/>
