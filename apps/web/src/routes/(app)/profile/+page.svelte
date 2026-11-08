<script lang="ts">
  // The signed-in user's own settings: profile, language and theme, password,
  // linked sign-in accounts and storage use.
  import { onMount } from 'svelte';
  import { resolve } from '$app/paths';
  import type { IdentityInfo, UpdateMeRequest } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Field,
    Input,
    ProviderIcon,
    Segmented,
    Skeleton,
    Tag,
    confirmAction,
    formatBytes,
    formatRelative,
    getThemeMode,
    setThemeMode,
    toast,
    type ThemeMode
  } from '@cloudit/ui';
  import { LANGUAGES, getLang, t, type Lang } from '@cloudit/ui/i18n';
  import {
    changePassword,
    listIdentities,
    startOAuth,
    unlinkIdentity,
    updateMe
  } from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import ProviderButtons from '#lib/components/ProviderButtons.svelte';
  import { errorText, formError } from '#lib/errors.ts';
  import { changeLang } from '#lib/lang.ts';
  import { session } from '#lib/session.svelte.ts';

  const user = $derived(session.user);
  const lang = $derived(getLang());

  // Profile
  let displayName = $state('');
  let email = $state('');
  let currentPassword = $state('');
  let profileBusy = $state(false);
  let profileError = $state<{ field?: string; form?: string }>({});

  $effect(() => {
    displayName = user?.display_name ?? '';
    email = user?.email ?? '';
  });

  const emailChanged = $derived(email.trim().toLowerCase() !== user?.email);

  async function saveProfile(event: SubmitEvent) {
    event.preventDefault();
    if (!user) return;
    profileError = {};
    const body: UpdateMeRequest = {};
    if (displayName.trim() !== user.display_name) body.display_name = displayName;
    if (emailChanged) {
      body.email = email;
      if (user.has_password) body.current_password = currentPassword;
    }
    if (Object.keys(body).length === 0) return;
    profileBusy = true;
    try {
      session.signedInAs(await updateMe(body));
      currentPassword = '';
      toast.success(t('profile.saved'));
    } catch (err) {
      profileError = formError(err, ['display_name', 'email', 'current_password']);
    } finally {
      profileBusy = false;
    }
  }

  // Password
  let oldPassword = $state('');
  let newPassword = $state('');
  let repeat = $state('');
  let passwordBusy = $state(false);
  let passwordError = $state<{ field?: string; form?: string }>({});

  async function savePassword(event: SubmitEvent) {
    event.preventDefault();
    if (!user) return;
    passwordError = {};
    if (newPassword.length < 8) return void (passwordError = { field: 'new_password' });
    if (newPassword !== repeat) return void (passwordError = { field: 'repeat' });
    passwordBusy = true;
    try {
      await changePassword({
        new_password: newPassword,
        current_password: user.has_password ? oldPassword : undefined
      });
      session.signedInAs({ ...user, has_password: true });
      oldPassword = newPassword = repeat = '';
      toast.success(t('profile.password_changed'));
    } catch (err) {
      passwordError = formError(err, ['current_password', 'new_password']);
    } finally {
      passwordBusy = false;
    }
  }

  // Linked accounts
  let identities = $state<IdentityInfo[] | null>(null);
  let identityError = $state<string | null>(null);

  async function loadIdentities() {
    try {
      identities = await listIdentities();
    } catch (err) {
      identityError = errorText(err);
    }
  }

  onMount(() => {
    void loadIdentities();
    void app.loadUsage().catch(() => undefined);
  });

  const linkable = $derived(
    (session.config?.providers ?? []).filter(
      (provider) => !identities?.some((identity) => identity.provider.id === provider.id)
    )
  );

  async function unlink(identity: IdentityInfo) {
    const ok = await confirmAction({
      title: t('identities.unlink_title', { provider: identity.provider.display_name }),
      message: t('identities.unlink_body'),
      confirmLabel: t('identities.unlink'),
      danger: true
    });
    if (!ok) return;
    identityError = null;
    try {
      await unlinkIdentity(identity.id);
      toast.success(t('identities.unlinked'));
    } catch (err) {
      identityError = errorText(err);
    }
    await loadIdentities();
  }

  async function link(provider: { id: string }) {
    identityError = null;
    try {
      const { redirect_url } = await startOAuth(provider.id, {
        purpose: 'link',
        return_to: resolve('/(app)/profile')
      });
      window.location.assign(redirect_url);
    } catch (err) {
      identityError = errorText(err);
    }
  }

  // Preferences
  const themes = $derived<{ value: ThemeMode; label: string }[]>([
    { value: 'system', label: t('theme.system') },
    { value: 'light', label: t('theme.light') },
    { value: 'dark', label: t('theme.dark') }
  ]);
  const languages = $derived(LANGUAGES.map((value) => ({ value, label: t(`language.${value}`) })));

  const usage = $derived(app.usage);
  const percent = $derived(
    usage?.quota_bytes
      ? Math.min(100, Math.round((usage.used_bytes / usage.quota_bytes) * 100))
      : null
  );
</script>

<svelte:head><title>{t('profile.title')} · {t('app.name')}</title></svelte:head>

<div class="mb-[18px]">
  <h1>{t('profile.title')}</h1>
  <p class="text-muted">{t('profile.subtitle')}</p>
</div>

{#if user}
  <div class="grid max-w-[760px] gap-5">
    <section class="card p-5" aria-labelledby="p-account">
      <h2 id="p-account" class="mb-3">{t('profile.account')}</h2>
      <form onsubmit={saveProfile} class="flex flex-col gap-3.5">
        {#if profileError.form}<Banner variant="danger">{profileError.form}</Banner>{/if}
        <Field
          label={t('field.name')}
          error={profileError.field === 'display_name' ? t('field.name_invalid') : undefined}
        >
          {#snippet children(control)}
            <Input
              {...control}
              bind:value={displayName}
              required
              maxlength={100}
              autocomplete="name"
            />
          {/snippet}
        </Field>
        <Field
          label={t('field.email')}
          error={profileError.field === 'email' ? t('field.email_invalid') : undefined}
        >
          {#snippet children(control)}
            <Input {...control} type="email" bind:value={email} required autocomplete="email" />
          {/snippet}
        </Field>
        <p class="hint -mt-2">
          <Tag variant={user.email_verified ? 'success' : 'neutral'}>
            {user.email_verified ? t('profile.verified') : t('profile.unverified')}
          </Tag>
        </p>
        {#if emailChanged && user.has_password}
          <Field
            label={t('profile.current_password')}
            hint={t('profile.current_password_hint')}
            error={profileError.field === 'current_password'
              ? t('error.invalid_credentials')
              : undefined}
          >
            {#snippet children(control)}
              <Input
                {...control}
                type="password"
                bind:value={currentPassword}
                required
                autocomplete="current-password"
              />
            {/snippet}
          </Field>
        {/if}
        <div>
          <Button variant="primary" type="submit" loading={profileBusy}>{t('common.save')}</Button>
        </div>
      </form>
    </section>

    <section class="card p-5" aria-labelledby="p-prefs">
      <h2 id="p-prefs" class="mb-3">{t('profile.preferences')}</h2>
      <div class="flex flex-wrap gap-x-8 gap-y-4">
        <Field label={t('language.label')} group>
          <Segmented
            name="pref-lang"
            options={languages}
            bind:value={() => lang, (value: Lang) => changeLang(value)}
          />
        </Field>
        <Field label={t('profile.theme')} group>
          <Segmented
            name="pref-theme"
            options={themes}
            bind:value={() => getThemeMode(), (value: ThemeMode) => setThemeMode(value)}
          />
        </Field>
      </div>
    </section>

    <section class="card p-5" aria-labelledby="p-password">
      <h2 id="p-password" class="mb-1">{t('profile.password')}</h2>
      <p class="mb-3 text-muted">
        {user.has_password ? t('profile.password_change_desc') : t('profile.password_set_desc')}
      </p>
      <form onsubmit={savePassword} class="flex flex-col gap-3.5">
        {#if passwordError.form}<Banner variant="danger">{passwordError.form}</Banner>{/if}
        {#if user.has_password}
          <Field
            label={t('profile.current_password')}
            error={passwordError.field === 'current_password'
              ? t('error.invalid_credentials')
              : undefined}
          >
            {#snippet children(control)}
              <Input
                {...control}
                type="password"
                bind:value={oldPassword}
                required
                autocomplete="current-password"
              />
            {/snippet}
          </Field>
        {/if}
        <Field
          label={t('field.new_password')}
          hint={t('field.password_hint')}
          error={passwordError.field === 'new_password' ? t('field.password_invalid') : undefined}
        >
          {#snippet children(control)}
            <Input
              {...control}
              type="password"
              bind:value={newPassword}
              required
              autocomplete="new-password"
            />
          {/snippet}
        </Field>
        <Field
          label={t('field.confirm_password')}
          error={passwordError.field === 'repeat' ? t('reset.mismatch') : undefined}
        >
          {#snippet children(control)}
            <Input
              {...control}
              type="password"
              bind:value={repeat}
              required
              autocomplete="new-password"
            />
          {/snippet}
        </Field>
        <div>
          <Button variant="primary" type="submit" loading={passwordBusy}>
            {user.has_password ? t('profile.password_change') : t('profile.password_set')}
          </Button>
        </div>
      </form>
    </section>

    {#if identities?.length || linkable.length || identityError}
      <section class="card p-5" aria-labelledby="p-identities">
        <h2 id="p-identities" class="mb-1">{t('profile.identities')}</h2>
        <p class="mb-3 text-muted">{t('profile.identities_desc')}</p>
        {#if identityError}<Banner variant="danger" class="mb-3">{identityError}</Banner>{/if}
        {#if identities === null}
          <Skeleton class="h-10 w-full" />
        {:else if identities.length === 0}
          <p class="text-muted">{t('identities.none')}</p>
        {:else}
          <ul class="divide-y divide-border">
            {#each identities as identity (identity.id)}
              <li class="flex items-center gap-3 py-2.5">
                <ProviderIcon icon={identity.provider.icon} kind={identity.provider.kind} />
                <div class="min-w-0 flex-1">
                  <b class="block">{identity.provider.display_name}</b>
                  <span class="block truncate text-xs text-muted">
                    {identity.email ?? ''}
                    {#if identity.last_login_at}
                      · {t('identities.last_login', {
                        when: formatRelative(identity.last_login_at, lang)
                      })}
                    {/if}
                  </span>
                </div>
                {#if !identity.provider.enabled}
                  <Tag variant="neutral">{t('identities.disabled')}</Tag>
                {/if}
                {#if identity.provider.deleted}
                  <Tag variant="neutral">{t('identities.deleted')}</Tag>
                {/if}
                <Button size="sm" variant="danger" onclick={() => unlink(identity)}>
                  {t('identities.unlink')}
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if linkable.length > 0}
          <div class="mt-3.5">
            <ProviderButtons
              providers={linkable}
              onstart={link}
              label={(name) => t('identities.link', { provider: name })}
            />
          </div>
        {/if}
      </section>
    {/if}

    <section class="card p-5" aria-labelledby="p-storage">
      <h2 id="p-storage" class="mb-3">{t('profile.storage')}</h2>
      {#if !usage}
        <Skeleton class="h-16 w-full" />
      {:else}
        <p class="font-semibold">
          {usage.quota_bytes === null
            ? t('storage.total_unlimited', { used: formatBytes(usage.used_bytes, lang) })
            : t('storage.total', {
                used: formatBytes(usage.used_bytes, lang),
                quota: formatBytes(usage.quota_bytes, lang)
              })}
        </p>
        {#if percent !== null}
          <div
            class="my-2 h-1.5 overflow-hidden rounded-full bg-border"
            role="meter"
            aria-label={t('profile.storage')}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          >
            <i class="block h-full bg-primary" style:width="{percent}%"></i>
          </div>
        {/if}
        {#if usage.scopes.length === 0}
          <p class="mt-2 text-muted">{t('storage.none')}</p>
        {:else}
          <ul class="mt-3 divide-y divide-border">
            {#each usage.scopes as scope (scope.id)}
              <li class="flex items-center gap-2 py-2">
                <span class="flex-1 truncate">{scope.name}</span>
                {#if scope.trashed}<Tag variant="neutral">{t('storage.trashed')}</Tag>{/if}
                <span class="text-muted tabular-nums">{formatBytes(scope.stored_bytes, lang)}</span>
              </li>
            {/each}
          </ul>
        {/if}
        <p class="hint mt-3">{t('storage.note')}</p>
      {/if}
    </section>
  </div>
{/if}
