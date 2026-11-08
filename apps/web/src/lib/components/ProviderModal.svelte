<script lang="ts">
  // Add or edit an identity provider. Adding starts with the preset grid, then the
  // preset's form; editing opens the form of the existing provider (id and kind are
  // fixed, a blank client secret keeps the stored one).
  import type {
    CreateProviderRequest,
    NewAccounts,
    ProviderInfo,
    ProviderPreset,
    UpdateProviderRequest
  } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Field,
    Input,
    Modal,
    ProviderIcon,
    RadioCards,
    Select,
    Toggle
  } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { formatList, isProviderId, parseList, slugify } from '#lib/admin.ts';
  import { ApiError, callbackUrl, createProvider, updateProvider } from '#lib/api.ts';
  import CopyField from './CopyField.svelte';
  import { errorText } from '#lib/errors.ts';
  import { resolve } from '$app/paths';

  const FIELDS = [
    'id',
    'display_name',
    'icon',
    'issuer_url',
    'tenant',
    'client_id',
    'client_secret',
    'scopes',
    'groups_claim',
    'admin_groups',
    'allowed_groups'
  ] as const;
  type FieldName = (typeof FIELDS)[number];

  let {
    open = $bindable(false),
    presets,
    editing,
    publicUrl,
    onsaved
  }: {
    open?: boolean;
    presets: ProviderPreset[];
    /** The provider being edited, or `null` to add one. */
    editing: ProviderInfo | null;
    publicUrl: string | null;
    onsaved: () => Promise<void>;
  } = $props();

  let preset = $state<ProviderPreset | null>(null);
  let id = $state('');
  let idTouched = $state(false);
  let displayName = $state('');
  let icon = $state('generic');
  let issuer = $state('');
  let tenant = $state('');
  let clientId = $state('');
  let clientSecret = $state('');
  let scopes = $state('');
  let groupsClaim = $state('');
  let adminGroups = $state('');
  let allowedGroups = $state('');
  let newAccounts = $state<NewAccounts>('approval');
  let enabled = $state(true);

  let busy = $state(false);
  let formMessage = $state<string | null>(null);
  let errors = $state<Partial<Record<FieldName, string>>>({});

  function startForm(p: ProviderPreset, provider: ProviderInfo | null) {
    preset = p;
    formMessage = null;
    errors = {};
    clientSecret = '';
    if (provider) {
      id = provider.id;
      displayName = provider.display_name;
      icon = provider.icon;
      issuer = provider.issuer_url ?? '';
      tenant = provider.tenant ?? '';
      clientId = provider.client_id;
      scopes = provider.scopes;
      groupsClaim = provider.groups_claim ?? '';
      adminGroups = formatList(provider.admin_groups);
      allowedGroups = formatList(provider.allowed_groups);
      newAccounts = provider.new_accounts;
      enabled = provider.enabled;
    } else {
      id = slugify(p.kind);
      idTouched = false;
      displayName = p.display_name;
      icon = p.icon;
      issuer = p.issuer_url ?? '';
      tenant = '';
      clientId = '';
      scopes = p.default_scopes;
      groupsClaim = '';
      adminGroups = allowedGroups = '';
      newAccounts = p.new_accounts;
      enabled = true;
    }
  }

  // (Re)start whenever the dialog opens.
  $effect(() => {
    if (!open) return;
    if (editing) {
      const p = presets.find((item) => item.kind === editing.kind);
      if (p) startForm(p, editing);
    } else {
      preset = null;
    }
  });

  const iconOptions = $derived(
    [...new Set([...presets.map((p) => p.icon), 'generic'])].map((key) => ({
      value: key,
      label: key
    }))
  );

  const callback = $derived(
    editing?.callback_url ?? (publicUrl && isProviderId(id) ? callbackUrl(publicUrl, id) : null)
  );

  const uid = $props.id();
  // Read inside snippets, where `preset` is not narrowed.
  const kind = $derived(preset?.kind);
  const issuerRequired = $derived(preset?.issuer_required ?? false);

  function pickName(next: string) {
    displayName = next;
    if (!editing && !idTouched) id = slugify(next);
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!preset) return;
    formMessage = null;
    const invalid: Partial<Record<FieldName, string>> = {};
    if (!editing && !isProviderId(id)) invalid.id = t('admin.provider_id_invalid');
    if (!displayName.trim()) invalid.display_name = t('field.name_invalid');
    if (preset.issuer_required && !issuer.trim()) invalid.issuer_url = t('admin.provider_required');
    if (preset.tenant_required && !tenant.trim()) invalid.tenant = t('admin.provider_required');
    if (!clientId.trim()) invalid.client_id = t('admin.provider_required');
    if (!editing && !clientSecret) invalid.client_secret = t('admin.provider_required');
    if (Object.keys(invalid).length > 0) {
      errors = invalid;
      return;
    }
    errors = {};

    const groups = preset.supports_groups
      ? {
          groups_claim: groupsClaim.trim(),
          admin_groups: parseList(adminGroups),
          allowed_groups: parseList(allowedGroups)
        }
      : {};
    busy = true;
    try {
      if (editing) {
        const body: UpdateProviderRequest = {
          display_name: displayName.trim(),
          icon,
          client_id: clientId.trim(),
          scopes: scopes.trim(),
          new_accounts: newAccounts,
          enabled,
          ...groups
        };
        if (preset.issuer_editable) body.issuer_url = issuer.trim();
        if (preset.tenant_required) body.tenant = tenant.trim();
        if (clientSecret) body.client_secret = clientSecret;
        await updateProvider(editing.id, body);
      } else {
        const body: CreateProviderRequest = {
          id,
          kind: preset.kind,
          display_name: displayName.trim(),
          icon,
          client_id: clientId.trim(),
          client_secret: clientSecret,
          scopes: scopes.trim(),
          new_accounts: newAccounts,
          enabled,
          ...groups
        };
        if (preset.issuer_editable && issuer.trim()) body.issuer_url = issuer.trim();
        if (preset.tenant_required) body.tenant = tenant.trim();
        await createProvider(body);
      }
      await onsaved();
      open = false;
    } catch (err) {
      if (err instanceof ApiError && err.code === 'provider_id_taken') {
        errors = { id: t('error.provider_id_taken') };
      } else if (
        err instanceof ApiError &&
        err.code === 'validation_failed' &&
        FIELDS.some((name) => name === err.field)
      ) {
        errors = { [err.field as FieldName]: t('error.validation_failed') };
      } else {
        formMessage = errorText(err);
      }
    } finally {
      busy = false;
    }
  }

  const title = $derived(
    editing
      ? t('admin.provider_edit_title', { name: editing.display_name })
      : preset
        ? t('admin.provider_add_kind', { name: preset.display_name })
        : t('admin.provider_add')
  );
</script>

<Modal bind:open {title} size="lg" dismissible={!busy}>
  {#if !preset}
    <p class="mb-3 text-muted">{t('admin.provider_pick')}</p>
    <ul class="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
      {#each presets as p (p.kind)}
        <li>
          <button
            type="button"
            class="card flex w-full cursor-pointer items-center gap-3 p-3 text-left hover:bg-surface-alt focus-visible:outline-2 focus-visible:outline-focus"
            data-preset={p.kind}
            onclick={() => startForm(p, null)}
          >
            <ProviderIcon icon={p.icon} kind={p.kind} size={28} />
            <span class="min-w-0">
              <span class="block truncate font-semibold">{p.display_name}</span>
              <span class="block text-xs text-muted">
                {p.protocol === 'oidc' ? 'OpenID Connect' : 'OAuth 2'}
              </span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <form id="provider-{uid}" onsubmit={submit} class="flex flex-col">
      {#if formMessage}<Banner variant="danger">{formMessage}</Banner>{/if}
      {#if !publicUrl}
        <Banner variant="warning" title={t('admin.provider_no_url')}>
          {t('admin.provider_no_url_body')}
          {#snippet actions()}
            <Button
              size="sm"
              href={resolve('/(app)/admin/settings')}
              onclick={() => (open = false)}
            >
              {t('admin.tab_settings')}
            </Button>
          {/snippet}
        </Banner>
      {/if}

      <div class="grid gap-x-4 sm:grid-cols-2">
        <Field label={t('admin.provider_name')} error={errors.display_name}>
          {#snippet children(control)}
            <Input
              {...control}
              value={displayName}
              oninput={(event) => pickName(event.currentTarget.value)}
              maxlength={100}
              autocomplete="off"
            />
          {/snippet}
        </Field>
        <Field
          label={t('admin.provider_id')}
          error={errors.id}
          hint={editing ? t('admin.provider_id_fixed') : t('admin.provider_id_hint')}
        >
          {#snippet children(control)}
            <Input
              {...control}
              bind:value={id}
              oninput={() => (idTouched = true)}
              disabled={editing !== null}
              maxlength={32}
              autocomplete="off"
              spellcheck="false"
              class="font-mono"
            />
          {/snippet}
        </Field>
      </div>

      <Field label={t('admin.provider_icon')}>
        {#snippet children(control)}
          <div class="flex items-center gap-2.5">
            <ProviderIcon {icon} {kind} size={28} />
            <Select {...control} bind:value={icon} options={iconOptions} class="max-w-48" />
          </div>
        {/snippet}
      </Field>

      {#if preset.issuer_editable}
        <Field
          label={t('admin.provider_issuer')}
          hint={t('admin.provider_issuer_hint')}
          error={errors.issuer_url}
        >
          {#snippet children(control)}
            <Input
              {...control}
              type="url"
              bind:value={issuer}
              required={issuerRequired}
              placeholder="https://auth.example.com/application/o/cloudit/"
              autocomplete="off"
            />
          {/snippet}
        </Field>
      {/if}
      {#if preset.tenant_required}
        <Field
          label={t('admin.provider_tenant')}
          hint={t('admin.provider_tenant_hint')}
          error={errors.tenant}
        >
          {#snippet children(control)}
            <Input {...control} bind:value={tenant} autocomplete="off" />
          {/snippet}
        </Field>
      {/if}

      <div class="grid gap-x-4 sm:grid-cols-2">
        <Field label={t('admin.provider_client_id')} error={errors.client_id}>
          {#snippet children(control)}
            <Input {...control} bind:value={clientId} autocomplete="off" spellcheck="false" />
          {/snippet}
        </Field>
        <Field
          label={t('admin.provider_client_secret')}
          error={errors.client_secret}
          hint={editing ? t('admin.provider_secret_keep') : undefined}
        >
          {#snippet children(control)}
            <Input
              {...control}
              type="password"
              bind:value={clientSecret}
              placeholder={editing ? '••••••••' : undefined}
              autocomplete="new-password"
            />
          {/snippet}
        </Field>
      </div>

      <Field label={t('admin.provider_scopes')} error={errors.scopes}>
        {#snippet children(control)}
          <Input {...control} bind:value={scopes} autocomplete="off" spellcheck="false" />
        {/snippet}
      </Field>

      {#if preset.supports_groups}
        <Field
          label={t('admin.provider_groups_claim')}
          hint={t('admin.provider_groups_claim_hint')}
          error={errors.groups_claim}
        >
          {#snippet children(control)}
            <Input
              {...control}
              bind:value={groupsClaim}
              placeholder="groups"
              autocomplete="off"
              spellcheck="false"
            />
          {/snippet}
        </Field>
        <div class="grid gap-x-4 sm:grid-cols-2">
          <Field
            label={t('admin.provider_admin_groups')}
            hint={t('admin.provider_admin_groups_hint')}
            error={errors.admin_groups}
          >
            {#snippet children(control)}
              <textarea {...control} class="input min-h-16" rows="2" bind:value={adminGroups}
              ></textarea>
            {/snippet}
          </Field>
          <Field
            label={t('admin.provider_allowed_groups')}
            hint={t('admin.provider_allowed_groups_hint')}
            error={errors.allowed_groups}
          >
            {#snippet children(control)}
              <textarea {...control} class="input min-h-16" rows="2" bind:value={allowedGroups}
              ></textarea>
            {/snippet}
          </Field>
        </div>
      {/if}

      <Field label={t('admin.provider_new_accounts')} group>
        <RadioCards
          name="new-accounts-{uid}"
          columns={2}
          bind:value={newAccounts}
          options={[
            {
              value: 'approval',
              title: t('admin.new_accounts_approval'),
              description: t('admin.new_accounts_approval_desc')
            },
            {
              value: 'auto',
              title: t('admin.new_accounts_auto'),
              description: t('admin.new_accounts_auto_desc')
            }
          ]}
        />
      </Field>

      <Toggle bind:checked={enabled} label={t('admin.provider_enabled')} />

      {#if callback}
        <Field label={t('admin.provider_callback')} hint={t('admin.provider_callback_hint')}>
          {#snippet children(control)}
            <CopyField value={callback} {control} />
          {/snippet}
        </Field>
      {/if}
    </form>
  {/if}
  {#snippet footer()}
    {#if preset && !editing}
      <Button onclick={() => (preset = null)} disabled={busy}>{t('admin.provider_back')}</Button>
    {:else}
      <Button onclick={() => (open = false)} disabled={busy}>{t('common.cancel')}</Button>
    {/if}
    {#if preset}
      <Button variant="primary" type="submit" form="provider-{uid}" loading={busy}>
        {editing ? t('common.save') : t('common.create')}
      </Button>
    {/if}
  {/snippet}
</Modal>
