<script lang="ts">
  // A password reset link handed out by an admin (or the CLI): one new password,
  // every session of the account is signed out.
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Banner, Button, Field, Input } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { ApiError, resetPassword } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import { errorText } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  const token = $derived(page.params.token ?? '');

  let password = $state('');
  let confirm = $state('');
  let busy = $state(false);
  let done = $state(false);
  let mismatch = $state(false);
  let passwordInvalid = $state(false);
  let formMessage = $state<string | null>(null);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    mismatch = password !== confirm;
    if (mismatch) return;
    busy = true;
    passwordInvalid = false;
    formMessage = null;
    try {
      await resetPassword({ token, new_password: password });
      // Every session was signed out, this browser's too.
      session.signedOut();
      done = true;
    } catch (err) {
      if (err instanceof ApiError && err.code === 'validation_failed') passwordInvalid = true;
      else formMessage = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>{t('reset.title')}</title></svelte:head>

{#if done}
  <AuthCard center title={t('reset.done_title')} subtitle={t('reset.done_body')}>
    <Button href={resolve('login')} variant="primary" block size="lg" class="mt-5">
      {t('reset.go_login')}
    </Button>
  </AuthCard>
{:else}
  <AuthCard title={t('reset.title')} subtitle={t('reset.subtitle')}>
    {#if formMessage}<Banner variant="danger" class="mt-5">{formMessage}</Banner>{/if}
    <form onsubmit={submit}>
      <Field
        label={t('field.new_password')}
        hint={t('field.password_hint')}
        error={passwordInvalid ? t('field.password_invalid') : undefined}
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
      <Field label={t('field.confirm_password')} error={mismatch ? t('reset.mismatch') : undefined}>
        {#snippet children(control)}
          <Input
            {...control}
            required
            type="password"
            autocomplete="new-password"
            bind:value={confirm}
          />
        {/snippet}
      </Field>
      <Button type="submit" variant="primary" size="lg" block class="mt-[18px]" loading={busy}>
        {t('reset.submit')}
      </Button>
    </form>
  </AuthCard>
{/if}
