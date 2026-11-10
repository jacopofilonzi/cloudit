<script lang="ts">
  // First run (and "sign in again"): the server address and the name of this computer,
  // then the browser does the sign-in while this page waits.
  import { onMount } from 'svelte';
  import { Banner, Button, Field, Input, Logo, Spinner } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { commandErrorText, errorText } from '#lib/errors.ts';

  let server = $state('');
  let device = $state('');
  let phase = $state<'form' | 'waiting'>('form');
  let starting = $state(false);
  let error = $state<string | null>(null);
  let waitingFor = $state('');

  onMount(async () => {
    server = app.account?.server_url ?? '';
    device = app.account?.device_name ?? (await api.suggestDeviceName());
  });

  // The browser sign-in ended without success: back to the form with the reason.
  $effect(() => {
    const event = app.login;
    if (phase === 'waiting' && event?.result === 'failed') {
      phase = 'form';
      if (event.error.kind !== 'cancelled')
        error = commandErrorText(new api.CommandError(event.error));
      app.login = null;
    }
  });

  async function start(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    starting = true;
    app.login = null;
    try {
      const url = await api.normalizeServerUrl(server);
      server = url;
      await api.startLogin(url, device);
      waitingFor = new URL(url).host;
      phase = 'waiting';
    } catch (err) {
      error = errorText(err);
    } finally {
      starting = false;
    }
  }

  async function cancel() {
    await api.cancelLogin();
    phase = 'form';
  }
</script>

<div class="grid h-screen place-items-center overflow-auto p-6">
  <div class="w-full max-w-[400px]">
    {#if phase === 'form'}
      <Logo size={44} />
      <h1 class="mt-3 mb-1 !text-[20px]">{t('desktop.welcome.title')}</h1>
      <p class="text-muted">{t('desktop.welcome.subtitle')}</p>

      {#if app.sessionEnded}
        <Banner variant="warning" class="mt-4">{t('desktop.welcome.ended')}</Banner>
      {/if}
      {#if error}
        <Banner variant="danger" class="mt-4">{error}</Banner>
      {/if}

      <form class="mt-4 space-y-3" onsubmit={start}>
        <Field label={t('desktop.welcome.server')} hint={t('desktop.welcome.server_hint')}>
          {#snippet children(control)}
            <Input
              {...control}
              bind:value={server}
              placeholder={t('desktop.welcome.server_placeholder')}
              autocomplete="url"
              spellcheck="false"
              required
            />
          {/snippet}
        </Field>
        <Field label={t('desktop.welcome.device')} hint={t('desktop.welcome.device_hint')}>
          {#snippet children(control)}
            <Input {...control} bind:value={device} maxlength={100} required />
          {/snippet}
        </Field>
        <Button type="submit" variant="primary" size="lg" block loading={starting}>
          {t('desktop.welcome.submit')}
        </Button>
      </form>
    {:else}
      <div class="text-center">
        <Spinner size={38} />
        <h1 class="mt-4 mb-1 !text-[20px]">{t('desktop.waiting.title')}</h1>
        <p class="text-muted">{t('desktop.waiting.body', { server: waitingFor })}</p>
        <div class="mt-5 flex justify-center gap-2">
          <Button onclick={() => api.reopenLoginBrowser()}>{t('desktop.waiting.reopen')}</Button>
          <Button variant="ghost" onclick={cancel}>{t('desktop.waiting.cancel')}</Button>
        </div>
      </div>
    {/if}
  </div>
</div>
