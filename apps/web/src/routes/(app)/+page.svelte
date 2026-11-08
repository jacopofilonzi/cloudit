<script lang="ts">
  // Placeholder home of milestone 5a; the scope view replaces it in 5b.
  import FolderSync from '@lucide/svelte/icons/folder-sync';
  import { EmptyState } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { session } from '#lib/session.svelte.ts';
</script>

<svelte:head><title>{t('app.name')}</title></svelte:head>

<div class="mb-[18px]">
  <h1>{t('home.welcome', { name: session.user?.display_name ?? '' })}</h1>
  <p class="text-muted">{t('app.tagline')}</p>
</div>

<EmptyState icon={FolderSync} title={t('home.empty_title')} description={t('home.empty_body')} />
