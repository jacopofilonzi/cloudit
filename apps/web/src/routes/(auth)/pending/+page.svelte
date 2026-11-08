<script lang="ts">
  import Hourglass from '@lucide/svelte/icons/hourglass';
  import { resolve } from '$app/paths';
  import { Button } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import AuthCard from '#lib/components/AuthCard.svelte';
</script>

<svelte:head><title>{t('pending.title')}</title></svelte:head>

<AuthCard center title={t('pending.title')} subtitle={t('pending.body')}>
  {#snippet header()}
    <span class="mx-auto grid size-14 place-items-center rounded-full bg-warning-soft text-warning">
      <Hourglass size={26} aria-hidden="true" />
    </span>
  {/snippet}
  <Button href={resolve('login')} class="mt-5">{t('common.back_to_login')}</Button>
</AuthCard>
