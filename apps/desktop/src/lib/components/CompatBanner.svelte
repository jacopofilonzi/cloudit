<script lang="ts">
  // The server is too old for this app, so nothing syncs: only its administrator can fix
  // it ("Check again" after). An app too old for the server gets the full-window
  // UpdateRequired screen instead of the shell, so no banner here.
  import { Banner, Button, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { reportError } from '#lib/actions.ts';
  import { app } from '#lib/app.svelte.ts';
  import { compatText } from '#lib/errors.ts';

  const compat = $derived(app.compat);
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
</script>

{#if app.signedIn && compat.status === 'server_outdated'}
  <Banner variant="warning" class="mb-3" title={compatText(compat) ?? ''}>
    <p>{t('desktop.compat.server_outdated_body')}</p>
    {#snippet actions()}
      <Button size="sm" loading={checking} onclick={checkAgain}>
        {t('desktop.compat.check_again')}
      </Button>
    {/snippet}
  </Banner>
{/if}
