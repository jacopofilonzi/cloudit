<script lang="ts">
  // Open registration (registration mode `open`). 201 signs in (the guard then
  // leaves this page); 202 means the account waits for an admin → /pending.
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { Banner, Button, Field, Input } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { register } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import { formError } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const FIELDS = ['display_name', 'email', 'password'] as const;
  type FieldName = (typeof FIELDS)[number];

  const open = $derived(session.config?.registration_mode === 'open');
  const approval = $derived(session.config?.password_requires_approval ?? false);

  let name = $state('');
  let email = $state('');
  let password = $state('');
  let busy = $state(false);
  let fieldError = $state<FieldName | null>(null);
  let formMessage = $state<string | null>(null);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    fieldError = null;
    formMessage = null;
    try {
      const result = await register({ display_name: name, email, password });
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

<svelte:head><title>{t('register.title')}</title></svelte:head>

{#if !open}
  <AuthCard title={t('register.closed_title')} subtitle={t('register.closed_body')}>
    <Button href={resolve('login')} block size="lg" class="mt-5">
      {t('common.back_to_login')}
    </Button>
  </AuthCard>
{:else}
  <AuthCard title={t('register.title')} subtitle={t('register.subtitle')}>
    {#if formMessage}<Banner variant="danger" class="mt-5">{formMessage}</Banner>{/if}
    <form onsubmit={submit}>
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
      {#if approval}<p class="note mt-3.5">{t('register.approval_note')}</p>{/if}
      <Button type="submit" variant="primary" size="lg" block class="mt-[18px]" loading={busy}>
        {t('register.submit')}
      </Button>
    </form>
    <p class="mt-[18px] text-center text-xs text-muted">
      {t('register.have_account')} <a href={resolve('login')}>{t('register.sign_in')}</a>
    </p>
  </AuthCard>
{/if}
