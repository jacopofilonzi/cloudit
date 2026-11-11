<script lang="ts">
  // A newer app version: the banner with its notes and the Update button (never
  // automatic), then the progress of the install the user asked for. Installed with a
  // package (.deb): a link to the release page instead; dev builds only show it.
  import { Banner, Button } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { reportError } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { downloadPercent } from '#lib/update.ts';

  const update = $derived(app.update);
  const version = $derived(update?.latest_version ?? '');
  const shown = $derived(
    !!update?.latest_version &&
      ['available', 'waiting', 'downloading', 'installing', 'failed'].includes(update.phase)
  );
  const percent = $derived(update ? downloadPercent(update) : null);
  let notesOpen = $state(false);
</script>

{#if update && shown}
  {#if update.phase === 'available'}
    <Banner variant="info" class="mb-3" title={t('desktop.update.available', { version })}>
      {#if update.install === 'release_page'}
        <p>{t('desktop.update.manual')}</p>
      {:else if update.install === 'never'}
        <p>{t('desktop.update.dev_build')}</p>
      {/if}
      {#if update.notes}
        <button
          type="button"
          class="mt-1 text-xs text-primary-fg hover:underline"
          aria-expanded={notesOpen}
          onclick={() => (notesOpen = !notesOpen)}
        >
          {t('desktop.update.notes')}
        </button>
        {#if notesOpen}
          <p class="mt-1 text-xs whitespace-pre-wrap text-muted">{update.notes}</p>
        {/if}
      {/if}
      {#snippet actions()}
        {#if update.install === 'in_app'}
          <Button
            size="sm"
            variant="primary"
            onclick={() => app.installUpdate().catch(reportError)}
          >
            {t('desktop.update.install')}
          </Button>
        {:else if update.install === 'release_page'}
          <Button size="sm" onclick={() => api.openReleasePage().catch(reportError)}>
            {t('desktop.update.download_page')}
          </Button>
        {/if}
      {/snippet}
    </Banner>
  {:else if update.phase === 'failed'}
    <Banner variant="warning" class="mb-3">
      {t('desktop.update.failed', { version })}
      {#snippet actions()}
        <Button size="sm" onclick={() => app.installUpdate().catch(reportError)}>
          {t('desktop.update.retry')}
        </Button>
      {/snippet}
    </Banner>
  {:else}
    <Banner variant="info" class="mb-3">
      {#if update.phase === 'waiting'}
        {t('desktop.update.waiting', { version })}
      {:else if update.phase === 'downloading'}
        {t('desktop.update.downloading', { version })}
        {#if percent !== null}
          <div
            class="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-alt"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          >
            <div class="h-full bg-primary" style:width="{percent}%"></div>
          </div>
        {/if}
      {:else}
        {t('desktop.update.installing', { version })}
      {/if}
    </Banner>
  {/if}
{/if}
