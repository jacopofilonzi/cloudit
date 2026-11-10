<script lang="ts">
  // First-run setup: the token from the server logs creates the first admin. The
  // public URL is prefilled from the browser's origin.
  import type { RegistrationMode } from '@cloudit/api-types';
  import { Banner, Button, Field, Input, Segmented } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { ApiError, completeSetup } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import { formError } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const FIELDS = ['token', 'display_name', 'email', 'password', 'public_url'] as const;
  type FieldName = (typeof FIELDS)[number];

  let token = $state('');
  let name = $state('');
  let email = $state('');
  let password = $state('');
  let publicUrl = $state(window.location.origin);
  let mode = $state<RegistrationMode>('invite');
  let busy = $state(false);
  let fieldError = $state<FieldName | null>(null);
  let formMessage = $state<string | null>(null);

  const modes = $derived([
    { value: 'open' as const, label: t('setup.mode_open') },
    { value: 'invite' as const, label: t('setup.mode_invite') }
  ]);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    fieldError = null;
    formMessage = null;
    try {
      const profile = await completeSetup({
        token: token.trim(),
        display_name: name,
        email,
        password,
        registration_mode: mode,
        public_url: publicUrl.trim() || undefined
      });
      session.signedInAs(profile);
    } catch (err) {
      if (err instanceof ApiError && err.code === 'setup_token_invalid') {
        fieldError = 'token';
      } else if (err instanceof ApiError && err.code === 'setup_already_done') {
        await session.load();
      } else {
        const { field, form } = formError(err, FIELDS);
        fieldError = field ?? null;
        formMessage = form ?? null;
      }
    } finally {
      busy = false;
    }
  }

  const fieldText: Record<FieldName, () => string> = {
    token: () => t('error.setup_token_invalid'),
    display_name: () => t('field.name_invalid'),
    email: () => t('field.email_invalid'),
    password: () => t('field.password_invalid'),
    public_url: () => t('setup.public_url_invalid')
  };
  const errorFor = (field: FieldName) => (fieldError === field ? fieldText[field]() : undefined);
</script>

<svelte:head><title>{t('setup.title')}</title></svelte:head>

<AuthCard wide title={t('setup.title')} subtitle={t('setup.subtitle')}>
  {#if formMessage}<Banner variant="danger" class="mt-5">{formMessage}</Banner>{/if}
  <form onsubmit={submit}>
    <Field label={t('setup.token')} error={errorFor('token')}>
      {#snippet hint()}
        <code>docker compose logs cloudit</code>
        {t('setup.token_hint_or')}
        <code>cloudit-server setup-token</code>
      {/snippet}
      {#snippet children(control)}
        <Input
          {...control}
          required
          autocomplete="off"
          spellcheck={false}
          placeholder={t('setup.token_placeholder')}
          bind:value={token}
        />
      {/snippet}
    </Field>
    <div class="grid gap-x-3 sm:grid-cols-2">
      <Field label={t('field.name')} error={errorFor('display_name')}>
        {#snippet children(control)}
          <Input {...control} required autocomplete="name" bind:value={name} />
        {/snippet}
      </Field>
      <Field label={t('field.email')} error={errorFor('email')}>
        {#snippet children(control)}
          <Input {...control} required type="email" autocomplete="email" bind:value={email} />
        {/snippet}
      </Field>
    </div>
    <Field label={t('field.password')} hint={t('field.password_hint')} error={errorFor('password')}>
      {#snippet children(control)}
        <Input
          {...control}
          required
          type="password"
          autocomplete="new-password"
          bind:value={password}
        />
      {/snippet}
    </Field>
    <Field
      label={t('setup.public_url')}
      hint={t('setup.public_url_hint')}
      error={errorFor('public_url')}
    >
      {#snippet children(control)}
        <Input {...control} type="url" spellcheck={false} bind:value={publicUrl} />
      {/snippet}
    </Field>
    <Field label={t('setup.registration')} group>
      <Segmented variant="chips" name="registration-mode" options={modes} bind:value={mode} />
    </Field>
    <p class="hint">{mode === 'open' ? t('setup.mode_open_hint') : t('setup.mode_invite_hint')}</p>
    <Button type="submit" variant="primary" size="lg" block class="mt-5" loading={busy}>
      {t('setup.submit')}
    </Button>
  </form>
</AuthCard>
