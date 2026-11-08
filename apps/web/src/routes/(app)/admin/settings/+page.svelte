<script lang="ts">
  // Server settings: registration, approvals, linking, public URL, quota, retention.
  import { onMount } from 'svelte';
  import type {
    NewAccounts,
    RegistrationMode,
    ServerSettings,
    UpdateSettingsRequest
  } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Card,
    Field,
    Input,
    RadioCards,
    Select,
    Skeleton,
    Toggle,
    toast
  } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { getSettings, updateSettings } from '#lib/api.ts';
  import { QUOTA_UNITS, parseIntIn, splitBytes, toBytes, type QuotaUnit } from '#lib/admin.ts';
  import { errorText, formError } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const FIELDS = [
    'registration_mode',
    'password_new_accounts',
    'public_url',
    'retention_max_versions',
    'retention_max_per_day',
    'trash_retention_days',
    'default_quota_bytes'
  ] as const;
  type FieldName = (typeof FIELDS)[number];

  let loaded = $state<ServerSettings | null>(null);
  let loadError = $state<string | null>(null);

  let registration = $state<RegistrationMode>('invite');
  let approval = $state(true);
  let autolink = $state(false);
  let publicUrl = $state('');
  let quotaMode = $state<'unlimited' | 'custom'>('unlimited');
  let quotaAmount = $state('');
  let quotaUnit = $state<QuotaUnit>('GB');
  let versions = $state('');
  let perDay = $state('');
  let trashDays = $state('');

  let busy = $state(false);
  let formMessage = $state<string | null>(null);
  let errors = $state<Partial<Record<FieldName, string>>>({});

  function fill(settings: ServerSettings) {
    loaded = settings;
    registration = settings.registration_mode;
    approval = settings.password_new_accounts === 'approval';
    autolink = settings.autolink_email;
    publicUrl = settings.public_url ?? '';
    if (settings.default_quota_bytes === null) {
      quotaMode = 'unlimited';
      quotaAmount = '';
    } else {
      quotaMode = 'custom';
      ({ amount: quotaAmount, unit: quotaUnit } = splitBytes(settings.default_quota_bytes));
    }
    versions = String(settings.retention_max_versions);
    perDay = String(settings.retention_max_per_day);
    trashDays = String(settings.trash_retention_days);
  }

  onMount(async () => {
    try {
      fill(await getSettings());
    } catch (err) {
      loadError = errorText(err);
    }
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!loaded) return;
    formMessage = null;
    errors = {};

    const body: UpdateSettingsRequest = {
      registration_mode: registration,
      password_new_accounts: (approval ? 'approval' : 'auto') satisfies NewAccounts,
      autolink_email: autolink
    };
    const invalid: Partial<Record<FieldName, string>> = {};
    const v = parseIntIn(versions, 0, 100000);
    const d = parseIntIn(perDay, 0, 100000);
    const days = parseIntIn(trashDays, 1, 3650);
    if (v === null) invalid.retention_max_versions = t('admin.settings_retention_invalid');
    else body.retention_max_versions = v;
    if (d === null) invalid.retention_max_per_day = t('admin.settings_retention_invalid');
    else body.retention_max_per_day = d;
    if (days === null) invalid.trash_retention_days = t('admin.settings_trash_invalid');
    else body.trash_retention_days = days;
    if (quotaMode === 'unlimited') body.default_quota_bytes = null;
    else {
      const bytes = toBytes(quotaAmount, quotaUnit);
      if (bytes === null) invalid.default_quota_bytes = t('admin.quota_invalid');
      else body.default_quota_bytes = bytes;
    }
    if (!loaded.public_url_from_env) body.public_url = publicUrl.trim();
    if (Object.keys(invalid).length > 0) {
      errors = invalid;
      return;
    }

    busy = true;
    try {
      fill(await updateSettings(body));
      await session.refreshConfig().catch(() => undefined);
      toast.success(t('common.saved'));
    } catch (err) {
      const failure = formError(err, FIELDS);
      if (failure.field) {
        errors = {
          [failure.field]:
            failure.field === 'public_url'
              ? t('setup.public_url_invalid')
              : t('error.validation_failed')
        };
      } else formMessage = failure.form ?? errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>{t('admin.tab_settings')} · {t('admin.title')} · {t('app.name')}</title>
</svelte:head>

{#if loadError}
  <Banner variant="danger">{loadError}</Banner>
{:else if loaded === null}
  <Skeleton class="h-64 w-full" />
{:else}
  <form onsubmit={save} class="flex max-w-[720px] flex-col gap-4">
    {#if formMessage}<Banner variant="danger">{formMessage}</Banner>{/if}

    <Card>
      <h2>{t('admin.settings_accounts')}</h2>
      <Field label={t('admin.settings_registration')} group error={errors.registration_mode}>
        <RadioCards
          name="registration"
          columns={2}
          bind:value={registration}
          options={[
            {
              value: 'invite',
              title: t('setup.mode_invite'),
              description: t('setup.mode_invite_hint')
            },
            { value: 'open', title: t('setup.mode_open'), description: t('setup.mode_open_hint') }
          ]}
        />
      </Field>
      <div class="mt-2 divide-y divide-border">
        <Toggle
          bind:checked={approval}
          label={t('admin.settings_approval')}
          description={t('admin.settings_approval_desc')}
        />
        <Toggle
          bind:checked={autolink}
          label={t('admin.settings_autolink')}
          description={t('admin.settings_autolink_desc')}
        />
      </div>
    </Card>

    <Card>
      <h2>{t('admin.settings_public_url')}</h2>
      <Field
        label={t('setup.public_url')}
        error={errors.public_url}
        hint={loaded.public_url_from_env
          ? t('admin.settings_public_url_env')
          : t('admin.settings_public_url_hint')}
      >
        {#snippet children(control)}
          <Input
            {...control}
            type="url"
            bind:value={publicUrl}
            disabled={loaded?.public_url_from_env}
            placeholder="https://sync.example.com"
            autocomplete="off"
          />
        {/snippet}
      </Field>
      <p class="mt-2 text-xs text-muted">
        {t('admin.settings_public_url_effective')}
        <span class="font-mono text-text">
          {loaded.effective_public_url ?? t('admin.settings_public_url_none')}
        </span>
      </p>
    </Card>

    <Card>
      <h2>{t('admin.settings_storage')}</h2>
      <Field label={t('admin.settings_quota')} group error={errors.default_quota_bytes}>
        <RadioCards
          name="default-quota"
          columns={2}
          bind:value={quotaMode}
          options={[
            { value: 'unlimited', title: t('admin.unlimited') },
            { value: 'custom', title: t('admin.quota_custom') }
          ]}
        />
        {#if quotaMode === 'custom'}
          <div class="mt-2.5 flex gap-2">
            <Input
              bind:value={quotaAmount}
              inputmode="decimal"
              autocomplete="off"
              aria-label={t('admin.quota_size')}
              class="min-w-0 flex-1"
            />
            <Select
              bind:value={quotaUnit}
              aria-label={t('admin.quota_unit')}
              options={QUOTA_UNITS.map((u) => ({ value: u, label: u }))}
              class="w-24"
            />
          </div>
        {/if}
      </Field>
      <p class="mt-2 text-xs text-muted">{t('admin.settings_quota_hint')}</p>
    </Card>

    <Card>
      <h2>{t('admin.settings_retention')}</h2>
      <p class="text-xs text-muted">{t('admin.settings_retention_desc')}</p>
      <div class="grid gap-x-4 sm:grid-cols-2">
        <Field label={t('admin.settings_versions')} error={errors.retention_max_versions}>
          {#snippet children(control)}
            <Input {...control} bind:value={versions} inputmode="numeric" autocomplete="off" />
          {/snippet}
        </Field>
        <Field label={t('admin.settings_per_day')} error={errors.retention_max_per_day}>
          {#snippet children(control)}
            <Input {...control} bind:value={perDay} inputmode="numeric" autocomplete="off" />
          {/snippet}
        </Field>
      </div>
      <Field
        label={t('admin.settings_trash_days')}
        hint={t('admin.settings_trash_hint')}
        error={errors.trash_retention_days}
      >
        {#snippet children(control)}
          <Input
            {...control}
            bind:value={trashDays}
            inputmode="numeric"
            autocomplete="off"
            class="sm:max-w-40"
          />
        {/snippet}
      </Field>
    </Card>

    <div class="flex justify-end">
      <Button variant="primary" type="submit" loading={busy}>{t('common.save')}</Button>
    </div>
  </form>
{/if}
