<script lang="ts">
  // Email + password and the enabled providers. A provider callback that failed
  // lands here with `?error=<code>`; `?return_to=` is where to go afterwards (the
  // root layout's guard follows it once signed in).
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Banner, Button, Field, Input } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { ApiError, login, startOAuth } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import ProviderButtons from '#lib/components/ProviderButtons.svelte';
  import { codeMessageKey, errorText } from '#lib/errors.ts';
  import { safeReturnTo } from '#lib/guard.ts';
  import { session } from '#lib/session.svelte.ts';

  const config = $derived(session.config);
  const providers = $derived(config?.providers ?? []);
  const returnTo = $derived(safeReturnTo(page.url.searchParams.get('return_to')));
  const callbackError = $derived(codeMessageKey(page.url.searchParams.get('error')));

  let email = $state('');
  let password = $state('');
  let busy = $state(false);
  let failure = $state<{ text: string; account: boolean } | null>(null);

  const ACCOUNT_STATES = ['account_pending', 'account_rejected', 'account_disabled'];

  function show(err: unknown) {
    failure = {
      text: errorText(err),
      account: err instanceof ApiError && ACCOUNT_STATES.includes(err.code)
    };
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    failure = null;
    try {
      session.signedInAs(await login({ email, password }));
    } catch (err) {
      show(err);
      password = '';
    } finally {
      busy = false;
    }
  }

  async function startProvider(provider: { id: string }) {
    failure = null;
    try {
      const { redirect_url } = await startOAuth(provider.id, {
        purpose: 'login',
        return_to: returnTo
      });
      window.location.assign(redirect_url);
    } catch (err) {
      show(err);
    }
  }
</script>

<svelte:head><title>{t('login.title')}</title></svelte:head>

<AuthCard title={t('login.title')} subtitle={t('login.subtitle')}>
  {#if callbackError && !failure}
    <Banner
      variant={page.url.searchParams.get('error') === 'account_pending' ? 'warning' : 'danger'}
      class="mt-5"
    >
      {t(callbackError)}
    </Banner>
  {/if}
  {#if failure}
    <Banner variant={failure.account ? 'warning' : 'danger'} class="mt-5">{failure.text}</Banner>
  {/if}

  <form onsubmit={submit}>
    <Field label={t('field.email')}>
      {#snippet children(control)}
        <Input {...control} type="email" autocomplete="username" required bind:value={email} />
      {/snippet}
    </Field>
    <Field label={t('field.password')}>
      {#snippet children(control)}
        <Input
          {...control}
          type="password"
          autocomplete="current-password"
          required
          bind:value={password}
        />
      {/snippet}
    </Field>
    <Button type="submit" variant="primary" size="lg" block class="mt-[18px]" loading={busy}>
      {t('login.submit')}
    </Button>
  </form>

  {#if providers.length > 0}
    <div class="divider-label">{t('common.or')}</div>
    <ProviderButtons {providers} onstart={startProvider} />
  {/if}

  <div class="mt-[18px] flex flex-col gap-1.5 text-center text-xs text-muted">
    {#if config?.registration_mode === 'open'}
      <p>
        {t('login.no_account')}
        <a href={resolve('register')}>{t('login.create_account')}</a>
      </p>
    {/if}
    <p>{t('login.forgot')}</p>
  </div>
</AuthCard>
