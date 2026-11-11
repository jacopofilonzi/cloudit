<script lang="ts">
  // Sets one account's storage quota: the server default, unlimited (offered only when
  // the default is limited) or a custom size.
  import type { AdminUser, AdminUserUpdate } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Field,
    Input,
    Modal,
    RadioCards,
    Select,
    formatBytes
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import {
    QUOTA_UNITS,
    offersUnlimited,
    quotaModeOf,
    quotaUpdate,
    splitBytes,
    toBytes,
    type QuotaMode,
    type QuotaUnit
  } from '#lib/admin.ts';
  import { errorText } from '#lib/errors.ts';

  let {
    open = $bindable(false),
    user,
    defaultQuota,
    onsave
  }: {
    open?: boolean;
    user: AdminUser | null;
    /** The server default (`null` = unlimited), shown as the meaning of "default". */
    defaultQuota: number | null;
    onsave: (update: Pick<AdminUserUpdate, 'quota_bytes' | 'quota_unlimited'>) => Promise<void>;
  } = $props();

  let mode = $state<QuotaMode>('default');
  // Fixed when the dialog opens, so the choice doesn't vanish while picking another one.
  let unlimitedOffered = $state(false);
  let amount = $state('');
  let unit = $state<QuotaUnit>('GB');
  let busy = $state(false);
  let error = $state<string | null>(null);
  let invalid = $state(false);

  $effect(() => {
    if (!open || !user) return;
    error = null;
    invalid = false;
    // A local value: reading `mode` here would make the effect reset every choice.
    const initial = quotaModeOf(user);
    mode = initial;
    unlimitedOffered = offersUnlimited(defaultQuota, initial);
    if (user.quota_bytes === null) {
      amount = '';
      unit = 'GB';
    } else {
      ({ amount, unit } = splitBytes(user.quota_bytes));
    }
  });

  const lang = $derived(getLang());
  const uid = $props.id();

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    invalid = false;
    let quota: number | null = null;
    if (mode === 'custom') {
      quota = toBytes(amount, unit);
      if (quota === null) return void (invalid = true);
    }
    busy = true;
    error = null;
    try {
      await onsave(quotaUpdate(mode, quota));
      open = false;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  bind:open
  title={t('admin.quota_title', { name: user?.display_name ?? '' })}
  description={t('admin.quota_desc')}
  dismissible={!busy}
>
  <form id="quota-{uid}" onsubmit={submit} class="flex flex-col">
    {#if error}<Banner variant="danger">{error}</Banner>{/if}
    <RadioCards
      name="quota-mode-{uid}"
      bind:value={mode}
      options={[
        {
          value: 'default',
          title: t('admin.quota_default'),
          description:
            defaultQuota === null
              ? t('admin.quota_default_unlimited')
              : t('admin.quota_default_of', { size: formatBytes(defaultQuota, lang) })
        },
        ...(unlimitedOffered
          ? [
              {
                value: 'unlimited' as const,
                title: t('admin.quota_unlimited'),
                description: t('admin.quota_unlimited_desc')
              }
            ]
          : []),
        { value: 'custom', title: t('admin.quota_custom') }
      ]}
    />
    {#if mode === 'custom'}
      <Field label={t('admin.quota_size')} error={invalid ? t('admin.quota_invalid') : undefined}>
        {#snippet children(control)}
          <div class="flex gap-2">
            <Input
              {...control}
              bind:value={amount}
              inputmode="decimal"
              autocomplete="off"
              class="min-w-0 flex-1"
            />
            <Select
              bind:value={unit}
              aria-label={t('admin.quota_unit')}
              options={QUOTA_UNITS.map((u) => ({ value: u, label: u }))}
              class="w-24"
            />
          </div>
        {/snippet}
      </Field>
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>{t('common.cancel')}</Button>
    <Button variant="primary" type="submit" form="quota-{uid}" loading={busy}>
      {t('common.save')}
    </Button>
  {/snippet}
</Modal>
