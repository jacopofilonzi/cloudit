<script lang="ts">
  // The server and this app don't work together, so nothing syncs: either the server
  // needs a newer app (offer the updater's Update when a version is out, else a check),
  // or the server is too old (only its administrator can fix it; "Check again" after).
  import { Banner, Button, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { reportError } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { compatText } from '#lib/errors.ts';

  const compat = $derived(app.compat);
  const update = $derived(app.update);
  const available = $derived(
    !!update?.latest_version && ['available', 'failed'].includes(update.phase)
  );
  let checking = $state(false);

  async function checkAgain(): Promise<void> {
    checking = true;
    try {
      const answer = await app.checkCompat();
      if (answer.status !== 'ok') toast.info(t('desktop.compat.still_outdated'));
    } catch (error) {
      reportError(error);
    } finally {
      checking = false;
    }
  }

  async function checkUpdates(): Promise<void> {
    checking = true;
    try {
      await app.checkUpdate();
    } catch (error) {
      reportError(error);
    } finally {
      checking = false;
    }
  }
</script>

{#if app.signedIn && compat.status === 'app_outdated'}
  <Banner variant="warning" class="mb-3" title={compatText(compat) ?? ''}>
    <p>{t('desktop.compat.app_outdated_body', { version: compat.app_version })}</p>
    {#snippet actions()}
      {#if available && update?.install === 'in_app'}
        <Button size="sm" variant="primary" onclick={() => app.installUpdate().catch(reportError)}>
          {t('desktop.update.install')}
        </Button>
      {:else if available && update?.install === 'release_page'}
        <Button size="sm" onclick={() => api.openReleasePage().catch(reportError)}>
          {t('desktop.update.download_page')}
        </Button>
      {:else if update?.phase !== 'waiting' && update?.phase !== 'downloading' && update?.phase !== 'installing'}
        <Button size="sm" loading={checking || update?.phase === 'checking'} onclick={checkUpdates}>
          {t('desktop.compat.check_updates')}
        </Button>
      {/if}
    {/snippet}
  </Banner>
{:else if app.signedIn && compat.status === 'server_outdated'}
  <Banner variant="warning" class="mb-3" title={compatText(compat) ?? ''}>
    <p>{t('desktop.compat.server_outdated_body')}</p>
    {#snippet actions()}
      <Button size="sm" loading={checking} onclick={checkAgain}>
        {t('desktop.compat.check_again')}
      </Button>
    {/snippet}
  </Banner>
{/if}
