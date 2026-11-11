<script lang="ts">
  // The whole window while the server needs a newer app: nothing else of the app can be
  // used (sync is stopped; the tray keeps running and says so). It shows the versions and
  // the updater: Update with its progress, the release page for a package install, a
  // note for development builds, else a check (one runs at once when no version is known).
  // Signing out stays possible, to use another server.
  import { onMount } from 'svelte';
  import { Banner, Button, Logo } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { confirmSignOut, reportError } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { checkOnEntry, requiredUpdate } from '#lib/update.ts';

  let { appVersion, minVersion }: { appVersion: string; minVersion: string } = $props();

  const view = $derived(requiredUpdate(app.update));
  let checking = $state(false);

  async function check(): Promise<void> {
    checking = true;
    try {
      await app.checkUpdate();
    } catch (error) {
      reportError(error);
    } finally {
      checking = false;
    }
  }

  function install(): void {
    app.installUpdate().catch(reportError);
  }

  onMount(() => {
    if (checkOnEntry(app.update)) void check();
  });
</script>

<div class="grid h-screen place-items-center overflow-auto p-6">
  <div class="w-full max-w-[440px]">
    <Logo size={44} />
    <h1 class="mt-3 mb-1 !text-[20px]">{t('desktop.required.title')}</h1>
    <p class="text-muted">{t('desktop.required.body')}</p>

    <div class="card mt-4 p-[18px]">
      <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
        <dt class="text-muted">{t('desktop.required.this_version')}</dt>
        <dd>{appVersion}</dd>
        <dt class="text-muted">{t('desktop.required.min_version')}</dt>
        <dd>{minVersion}</dd>
      </dl>

      <div class="mt-4 border-t border-border pt-4" aria-live="polite">
        {#if view.kind === 'install'}
          <p><b>{t('desktop.update.available', { version: view.version })}</b></p>
          <Button class="mt-3" variant="primary" block onclick={install}>
            {t('desktop.update.install')}
          </Button>
        {:else if view.kind === 'download_page'}
          <p><b>{t('desktop.update.available', { version: view.version })}</b></p>
          <p class="mt-1 text-xs text-muted">{t('desktop.update.manual')}</p>
          <Button
            class="mt-3"
            variant="primary"
            block
            onclick={() => api.openReleasePage().catch(reportError)}
          >
            {t('desktop.update.download_page')}
          </Button>
        {:else if view.kind === 'dev_build'}
          <p><b>{t('desktop.update.available', { version: view.version })}</b></p>
          <p class="mt-1 text-xs text-muted">{t('desktop.update.dev_build')}</p>
        {:else if view.kind === 'waiting' || view.kind === 'downloading' || view.kind === 'installing'}
          <p>
            {t(
              view.kind === 'waiting'
                ? 'desktop.update.waiting'
                : view.kind === 'downloading'
                  ? 'desktop.update.downloading'
                  : 'desktop.update.installing',
              { version: view.version }
            )}
          </p>
          {#if view.kind === 'downloading' && view.percent !== null}
            <div
              class="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-alt"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={view.percent}
            >
              <div class="h-full bg-primary" style:width="{view.percent}%"></div>
            </div>
          {/if}
        {:else if view.kind === 'failed'}
          <Banner variant="warning">{t('desktop.update.failed', { version: view.version })}</Banner>
          <Button class="mt-3" variant="primary" block onclick={install}>
            {t('desktop.update.retry')}
          </Button>
        {:else}
          <p class="text-muted">
            {view.kind === 'none' && view.checkFailed
              ? t('desktop.settings.check_failed')
              : view.kind === 'none'
                ? t('desktop.required.none')
                : t('desktop.required.checking')}
          </p>
          <Button class="mt-3" block loading={checking || view.kind === 'checking'} onclick={check}>
            {t('desktop.compat.check_again')}
          </Button>
        {/if}
      </div>
    </div>

    <p class="mt-4 text-xs text-muted">{t('desktop.required.tray')}</p>
    <div class="mt-3 flex items-center gap-3 text-xs text-muted">
      <span>{t('desktop.required.other_server')}</span>
      <Button class="ms-auto" variant="ghost" size="sm" onclick={confirmSignOut}>
        {t('desktop.settings.sign_out')}
      </Button>
    </div>
  </div>
</div>
