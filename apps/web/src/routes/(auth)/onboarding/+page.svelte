<script lang="ts">
  // After a provider sign-in with an identity no account knows yet: create a new
  // account from the provider's data, or attach the identity to an existing account
  // (its email + password, or another provider already linked to it).
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import type { OnboardingInfo } from '@cloudit/api-types';
  import { Banner, Button, Field, Input, ProviderIcon, RadioCards, Spinner } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import {
    ApiError,
    getOnboarding,
    onboardingCreate,
    onboardingLink,
    onboardingLinkStart
  } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import ProviderButtons from '#lib/components/ProviderButtons.svelte';
  import { errorText, formError } from '#lib/errors.ts';
  import { inviteToken } from '#lib/fields.ts';
  import { session } from '#lib/session.svelte.ts';

  const FIELDS = ['display_name', 'email', 'password', 'invite_token'] as const;
  type FieldName = (typeof FIELDS)[number];

  type Load =
    | { kind: 'loading' }
    | { kind: 'expired' }
    | { kind: 'error'; message: string }
    | { kind: 'ready'; info: OnboardingInfo };
  let load = $state<Load>({ kind: 'loading' });

  let choice = $state<'new' | 'link'>('new');
  let name = $state('');
  let email = $state('');
  let password = $state('');
  let invite = $state('');
  let busy = $state(false);
  let fieldError = $state<FieldName | null>(null);
  let formMessage = $state<string | null>(null);

  onMount(async () => {
    try {
      const info = await getOnboarding();
      name = info.display_name;
      email = info.email ?? '';
      load = { kind: 'ready', info };
    } catch (err) {
      load =
        err instanceof ApiError && err.code === 'onboarding_expired'
          ? { kind: 'expired' }
          : { kind: 'error', message: errorText(err) };
    }
  });

  function failed(err: unknown) {
    if (err instanceof ApiError && err.code === 'onboarding_expired') {
      load = { kind: 'expired' };
      return;
    }
    const { field, form } = formError(err, FIELDS);
    fieldError = field ?? null;
    formMessage = form ?? null;
  }

  async function create(info: OnboardingInfo, event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    fieldError = null;
    formMessage = null;
    try {
      const typedEmail = email.trim();
      const result = await onboardingCreate({
        display_name: name,
        email: typedEmail && typedEmail !== info.email ? typedEmail : undefined,
        password: password || undefined,
        invite_token: invite.trim() ? inviteToken(invite) : undefined
      });
      if (result.kind === 'pending') {
        await goto(resolve('pending'));
      } else {
        session.signedInAs(result.profile);
        await goto(resolve(''));
      }
    } catch (err) {
      failed(err);
    } finally {
      busy = false;
    }
  }

  async function link(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    fieldError = null;
    formMessage = null;
    try {
      session.signedInAs(await onboardingLink({ email, password }));
      await goto(resolve(''));
    } catch (err) {
      failed(err);
      password = '';
    } finally {
      busy = false;
    }
  }

  async function linkWith(provider: { id: string }) {
    formMessage = null;
    try {
      const { redirect_url } = await onboardingLinkStart(provider.id);
      window.location.assign(redirect_url);
    } catch (err) {
      failed(err);
    }
  }

  const fieldText: Record<FieldName, () => string> = {
    display_name: () => t('field.name_invalid'),
    email: () => t('field.email_invalid'),
    password: () => t('field.password_invalid'),
    invite_token: () => t('error.invite_invalid')
  };
  const errorFor = (field: FieldName) => (fieldError === field ? fieldText[field]() : undefined);

  function emailHint(info: OnboardingInfo): string {
    const provider = info.provider.display_name;
    if (!info.email_from_provider) return t('onboarding.email_required', { provider });
    return info.email_verified
      ? t('onboarding.email_verified', { provider })
      : t('onboarding.email_unverified', { provider });
  }
</script>

<svelte:head><title>{t('onboarding.title')}</title></svelte:head>

{#if load.kind === 'loading'}
  <Spinner size={22} label={t('common.loading')} />
{:else if load.kind === 'expired' || load.kind === 'error'}
  <AuthCard
    title={t('onboarding.expired_title')}
    subtitle={load.kind === 'expired' ? t('error.onboarding_expired') : load.message}
  >
    <Button href={resolve('login')} block size="lg" class="mt-5">
      {t('common.back_to_login')}
    </Button>
  </AuthCard>
{:else}
  {@const info = load.info}
  {@const provider = info.provider.display_name}
  {@const otherProviders = (session.config?.providers ?? []).filter(
    (p) => p.id !== info.provider.id
  )}
  <AuthCard wide title={t('onboarding.title')} subtitle={t('onboarding.subtitle', { provider })}>
    {#snippet header()}
      <div class="flex items-center gap-2.5">
        <ProviderIcon icon={info.provider.icon} kind={info.provider.kind} size={22} />
        <b>{t('onboarding.signed_in_with', { provider })}</b>
      </div>
    {/snippet}

    <RadioCards
      class="mt-[18px] mb-1.5"
      name="onboarding-choice"
      columns={2}
      bind:value={choice}
      options={[
        {
          value: 'new',
          title: t('onboarding.new'),
          description: t('onboarding.new_desc', { provider })
        },
        {
          value: 'link',
          title: t('onboarding.link'),
          description: t('onboarding.link_desc', { provider })
        }
      ]}
    />

    {#if formMessage}<Banner variant="danger" class="mt-4">{formMessage}</Banner>{/if}

    {#if choice === 'new'}
      <form onsubmit={(event) => create(info, event)}>
        <Field label={t('field.name')} error={errorFor('display_name')}>
          {#snippet children(control)}
            <Input {...control} required autocomplete="name" bind:value={name} />
          {/snippet}
        </Field>
        <Field label={t('field.email')} hint={emailHint(info)} error={errorFor('email')}>
          {#snippet children(control)}
            <Input {...control} required type="email" autocomplete="email" bind:value={email} />
          {/snippet}
        </Field>
        <Field label={t('onboarding.password_optional')} error={errorFor('password')}>
          {#snippet children(control)}
            <Input
              {...control}
              type="password"
              autocomplete="new-password"
              placeholder={t('onboarding.password_placeholder', { provider })}
              bind:value={password}
            />
          {/snippet}
        </Field>
        {#if info.invite_required}
          <Field
            label={t('onboarding.invite')}
            hint={t('onboarding.invite_hint')}
            error={errorFor('invite_token')}
          >
            {#snippet children(control)}
              <Input
                {...control}
                required
                autocomplete="off"
                spellcheck={false}
                bind:value={invite}
              />
            {/snippet}
          </Field>
        {/if}
        {#if info.requires_approval}
          <p class="note mt-3.5">{t('onboarding.approval_note', { provider })}</p>
        {/if}
        <Button type="submit" variant="primary" size="lg" block class="mt-4" loading={busy}>
          {t('onboarding.create')}
        </Button>
      </form>
    {:else}
      <form onsubmit={link}>
        <Field label={t('field.email')}>
          {#snippet children(control)}
            <Input {...control} required type="email" autocomplete="username" bind:value={email} />
          {/snippet}
        </Field>
        <Field label={t('field.password')}>
          {#snippet children(control)}
            <Input
              {...control}
              required
              type="password"
              autocomplete="current-password"
              bind:value={password}
            />
          {/snippet}
        </Field>
        <Button type="submit" variant="primary" size="lg" block class="mt-4" loading={busy}>
          {t('onboarding.link_submit', { provider })}
        </Button>
      </form>
      {#if otherProviders.length > 0}
        <div class="divider-label">{t('onboarding.link_other')}</div>
        <ProviderButtons providers={otherProviders} onstart={linkWith} />
      {/if}
    {/if}
  </AuthCard>
{/if}
