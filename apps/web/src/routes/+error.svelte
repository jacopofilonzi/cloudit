<script lang="ts">
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Button } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import AuthCard from '#lib/components/AuthCard.svelte';
</script>

<svelte:head><title>{t('errorpage.title')}</title></svelte:head>

<div class="grid min-h-screen place-items-center p-[30px]">
  <AuthCard
    title={page.status === 404 ? t('errorpage.not_found') : t('errorpage.title')}
    subtitle={page.status === 404 ? t('errorpage.not_found_body') : t('error.unknown')}
  >
    <Button href={resolve('')} class="mt-5">{t('errorpage.home')}</Button>
  </AuthCard>
</div>
