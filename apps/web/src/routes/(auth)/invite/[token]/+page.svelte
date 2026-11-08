<script lang="ts">
  // An invite link: who invited, until when, then a provider sign-in carrying the
  // invite or a password registration with it (the email is fixed when the invite
  // is bound to one).
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import type { InvitePreview } from '@cloudit/api-types';
  import { Banner, Button, Field, Input, Spinner, formatRelative } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { ApiError, lookupInvite, register, startOAuth } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import ProviderButtons from '#lib/components/ProviderButtons.svelte';
  import { errorText, formError } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const FIELDS = ['display_name', 'email', 'password'] as const;
  type FieldName = (typeof FIELDS)[number];

  const token = $derived(page.params.token ?? '');
  const providers = $derived(session.config?.providers ?? []);
  const host = window.location.host;

  type Lookup =
    | { kind: 'loading' }
    | { kind: 'invalid'; message: string }
    | { kind: 'ready'; invite: InvitePreview };
  let lookup = $state<Lookup>({ kind: 'loading' });

  let name = $state('');
  let email = $state('');
  let password = $state('');
  let busy = $state(false);
  let fieldError = $state<FieldName | null>(null);
  let formMessage = $state<string | null>(null);

  onMount(async () => {
    try {
      const invite = await lookupInvite(token);
      email = invite.email ?? '';
      lookup = { kind: 'ready', invite };
    } catch (err) {
      const invalid = err instanceof ApiError && err.code === 'invite_invalid';
      lookup = { kind: 'invalid', message: invalid ? t('invite.invalid_body') : errorText(err) };
    }
  });

  async function startProvider(provider: { id: string }) {
    formMessage = null;
    try {
      const { redirect_url } = await startOAuth(provider.id, {
        purpose: 'login',
        invite_token: token
      });
      window.location.assign(redirect_url);
    } catch (err) {
      formMessage = errorText(err);
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    fieldError = null;
    formMessage = null;
    try {
      const result = await register({ display_name: name, email, password, invite_token: token });
      if (result.kind === 'pending') await goto(resolve('pending'));
      else session.signedInAs(result.profile);
    } catch (err) {
      const { field, form } = formError(err, FIELDS);
      fieldError = field ?? null;
      formMessage = form ?? null;
    } finally {
      busy = false;
    }
  }

  const fieldText: Record<FieldName, () => string> = {
    display_name: () => t('field.name_invalid'),
    email: () => t('field.email_invalid'),
    password: () => t('field.password_invalid')
  };
  const errorFor = (field: FieldName) => (fieldError === field ? fieldText[field]() : undefined);
</script>

<svelte:head><title>{t('invite.title')}</title></svelte:head>

{#if lookup.kind === 'loading'}
  <Spinner size={22} label={t('invite.checking')} />
{:else if lookup.kind === 'invalid'}
  <AuthCard title={t('invite.invalid_title')} subtitle={lookup.message}>
    <Button href={resolve('login')} block size="lg" class="mt-5">
      {t('common.back_to_login')}
    </Button>
  </AuthCard>
{:else}
  {@const invite = lookup.invite}
  <AuthCard title={t('invite.title')}>
    {#snippet subtitle()}
      {invite.invited_by
        ? t('invite.body', { name: invite.invited_by, host })
        : t('invite.body_anonymous', { host })}
      {t('invite.expires', { when: formatRelative(invite.expires_at, getLang()) })}
    {/snippet}

    {#if formMessage}<Banner variant="danger" class="mt-5">{formMessage}</Banner>{/if}

    {#if providers.length > 0}
      <div class="mt-5">
        <ProviderButtons {providers} onstart={startProvider} />
      </div>
      <div class="divider-label">{t('invite.or_email')}</div>
    {/if}

    <form onsubmit={submit}>
      <Field label={t('field.name')} error={errorFor('display_name')}>
        {#snippet children(control)}
          <Input {...control} required autocomplete="name" bind:value={name} />
        {/snippet}
      </Field>
      <Field
        label={t('field.email')}
        hint={invite.email ? t('invite.email_bound') : undefined}
        error={errorFor('email')}
      >
        {#snippet children(control)}
          <Input
            {...control}
            required
            type="email"
            autocomplete="email"
            readonly={invite.email !== null}
            bind:value={email}
          />
        {/snippet}
      </Field>
      <Field
        label={t('field.password')}
        hint={t('field.password_hint')}
        error={errorFor('password')}
      >
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
      <Button type="submit" variant="primary" size="lg" block class="mt-[18px]" loading={busy}>
        {t('invite.submit')}
      </Button>
    </form>
  </AuthCard>
{/if}
