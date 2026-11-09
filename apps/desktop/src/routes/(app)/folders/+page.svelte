<script lang="ts">
  // Main screen: the linked folders as a list with the selected one's detail beside it.
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import FolderSync from '@lucide/svelte/icons/folder-sync';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { Banner, Button, EmptyState } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { reportError, showConflict } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import FolderDetail from '#lib/components/FolderDetail.svelte';
  import FolderRow from '#lib/components/FolderRow.svelte';

  const selected = $derived(
    app.folders.find((f) => f.id === app.selected) ?? app.folders[0] ?? null
  );
  const conflicts = $derived(app.conflicts);
  const syncing = $derived(Object.keys(app.busy).length > 0);

  async function syncAll() {
    try {
      await api.syncAll();
    } catch (error) {
      reportError(error);
    }
  }
</script>

<div class="flex h-full flex-col gap-3">
  <div class="flex items-center gap-2.5">
    <h1 class="!text-[19px]">{t('desktop.folders.title')}</h1>
    <div class="ms-auto flex gap-1.5">
      <Button disabled={app.folders.length === 0 || syncing} onclick={syncAll}>
        <RefreshCw size={14} aria-hidden="true" />{t('desktop.folders.sync_all')}
      </Button>
      <Button variant="primary" onclick={() => goto(resolve('/(app)/link'))}>
        <FolderPlus size={14} aria-hidden="true" />{t('desktop.folders.link')}
      </Button>
    </div>
  </div>

  {#if app.tokenWarning}
    <Banner variant="warning">{t('desktop.folders.token_warning')}</Banner>
  {/if}

  {#if conflicts.length > 0}
    <Banner variant="warning">
      {conflicts.length === 1 && conflicts[0]
        ? t('desktop.folders.banner_one', { name: conflicts[0].folder_name })
        : t('desktop.folders.banner_many', { count: conflicts.length })}
      {#snippet actions()}
        <Button size="sm" onclick={() => conflicts[0] && showConflict(conflicts[0].id)}>
          {t('desktop.folders.banner_action')}
        </Button>
      {/snippet}
    </Banner>
  {/if}

  {#if app.folders.length === 0}
    <EmptyState
      icon={FolderSync}
      title={t('desktop.folders.empty_title')}
      description={t('desktop.folders.empty_body')}
    >
      {#snippet action()}
        <Button variant="primary" onclick={() => goto(resolve('/(app)/link'))}>
          <FolderPlus size={14} aria-hidden="true" />{t('desktop.folders.link')}
        </Button>
      {/snippet}
    </EmptyState>
  {:else}
    <div class="grid min-h-0 grid-cols-[1fr_300px] items-start gap-3">
      <div class="card overflow-hidden">
        {#each app.folders as link (link.id)}
          <FolderRow
            {link}
            selected={selected?.id === link.id}
            onselect={() => (app.selected = link.id)}
          />
        {/each}
      </div>
      {#if selected}
        {#key selected.id}
          <FolderDetail link={selected} />
        {/key}
      {/if}
    </div>
    {#if !app.autoSync}
      <p class="text-xs text-muted">{t('desktop.folders.auto_off_note')}</p>
    {/if}
  {/if}
</div>
