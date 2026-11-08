<script lang="ts">
  // Placeholder: the administration pages come with milestone 5c.
  import Shield from '@lucide/svelte/icons/shield';
  import { EmptyState } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
</script>

<svelte:head><title>{t('admin.soon_title')} · {t('app.name')}</title></svelte:head>

<EmptyState icon={Shield} title={t('admin.soon_title')} description={t('admin.soon_body')} />
