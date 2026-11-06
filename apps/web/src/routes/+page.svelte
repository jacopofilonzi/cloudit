<script lang="ts">
  import { onMount } from 'svelte';
  import type { HealthResponse } from '@cloudit/api-types';
  import { Button } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { getHealth } from '#lib/api.ts';
  import { errorMessageKey } from '#lib/errors.ts';
  import type { MessageKey } from '@cloudit/ui/i18n';

  type Status =
    | { kind: 'loading' }
    | { kind: 'ok'; health: HealthResponse }
    | { kind: 'error'; key: MessageKey };

  let status = $state<Status>({ kind: 'loading' });

  async function check() {
    status = { kind: 'loading' };
    try {
      status = { kind: 'ok', health: await getHealth() };
    } catch (err) {
      status = { kind: 'error', key: errorMessageKey(err) };
    }
  }

  onMount(check);
</script>

<section class="card flex flex-col gap-3">
  <h2 class="text-lg font-medium">{t('health.title')}</h2>

  {#if status.kind === 'loading'}
    <p class="text-muted">{t('health.loading')}</p>
  {:else if status.kind === 'ok'}
    <p class="font-medium text-success">{t('health.ok')}</p>
    <p class="text-sm text-muted">{t('health.version', { version: status.health.version })}</p>
  {:else}
    <p class="text-danger">{t(status.key)}</p>
  {/if}

  <div>
    <Button variant="primary" onclick={check} disabled={status.kind === 'loading'}>
      {t('health.retry')}
    </Button>
  </div>
</section>
