<script lang="ts">
  // The selected folder: path, status, last sync, changed files, the main action for its
  // state, live progress with cancel, and links to the folder and its history.
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import History from '@lucide/svelte/icons/history';
  import Settings from '@lucide/svelte/icons/settings';
  import type { LinkedFolder } from '@cloudit/api-types';
  import { Banner, Button, formatRelative, formatNumber } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { runPrimary, showFolderSettings } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { commandErrorText, problemText } from '#lib/errors.ts';
  import {
    ACTION_LABEL,
    STATE_HINT,
    changedPaths,
    displayState,
    primaryAction
  } from '#lib/status.ts';
  import FolderIcon from './FolderIcon.svelte';
  import OpProgress from './OpProgress.svelte';
  import StatusTag from './StatusTag.svelte';

  let { link }: { link: LinkedFolder } = $props();

  const lang = $derived(getLang());
  const status = $derived(app.statuses[link.id]);
  const folderState = $derived(displayState(link, status));
  const action = $derived(primaryAction(folderState));
  const running = $derived(app.busy[link.id]);
  const error = $derived(app.errors[link.id]);
  let cancelling = $state(false);

  $effect(() => {
    if (running === undefined) cancelling = false;
  });

  async function cancel() {
    cancelling = true;
    try {
      await api.cancelOp(link.id);
    } catch {
      cancelling = false;
    }
  }

  const changes = $derived(
    status
      ? t('desktop.detail.changes_value', {
          local: formatNumber(status.local_changes.length, lang),
          remote: formatNumber(status.remote_changes.length, lang)
        })
      : null
  );
</script>

<div class="card flex flex-col gap-3 p-4">
  <div class="flex items-center gap-2.5">
    <FolderIcon emoji={link.emoji} />
    <div class="min-w-0">
      <b class="block truncate text-[15px]">{link.folder_name}</b>
      <span class="block truncate text-xs text-muted">{link.scope_name}</span>
    </div>
  </div>

  <div>
    <span class="field-label !mb-0.5">{t('desktop.detail.path')}</span>
    <div class="font-mono text-[11.5px] break-all text-muted">{link.local_path}</div>
  </div>

  <div class="grid grid-cols-2 gap-2">
    <div class="col-span-2 rounded-lg bg-surface-alt px-2.5 py-2">
      <span class="block text-xs text-muted">{t('desktop.detail.status')}</span>
      <StatusTag state={folderState} />
    </div>
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <span class="block text-xs text-muted">{t('desktop.detail.last_sync')}</span>
      <b class="block truncate">
        {link.last_sync_at
          ? formatRelative(link.last_sync_at, lang)
          : t('desktop.folders.never_synced')}
      </b>
    </div>
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <span class="block text-xs text-muted">{t('desktop.detail.changes')}</span>
      <b class="block">
        {#if status}
          {changedPaths(status) === 0 ? t('desktop.detail.none') : changes}
        {:else}
          –
        {/if}
      </b>
    </div>
  </div>

  <p class="text-xs text-muted">{t(STATE_HINT[folderState])}</p>

  {#if error}
    <Banner variant="danger" title={t('desktop.detail.last_error')}>
      {commandErrorText(error)}
    </Banner>
  {/if}

  {#if status && status.problems.length > 0}
    <div class="note">
      <b class="mb-1 block text-text">{t('desktop.detail.problems')}</b>
      <ul class="max-h-28 space-y-0.5 overflow-auto">
        {#each status.problems as problem (problem.path)}
          <li class="flex gap-1.5">
            <span class="min-w-0 truncate font-mono text-[11.5px]">{problem.path}</span>
            <span class="flex-none">· {problemText(problem)}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if running !== undefined}
    <div>
      <OpProgress progress={app.progress[link.id]} />
      <div class="mt-2">
        <Button size="sm" variant="secondary" disabled={cancelling} onclick={cancel}>
          {cancelling ? t('desktop.detail.cancelling') : t('desktop.detail.cancel')}
        </Button>
      </div>
    </div>
  {:else}
    <div class="flex gap-1.5">
      <Button variant="primary" class="flex-1" onclick={() => runPrimary(link)}>
        {t(ACTION_LABEL[action])}
      </Button>
      <Button
        variant="secondary"
        title={t('desktop.detail.recheck')}
        aria-label={t('desktop.detail.recheck')}
        onclick={() => api.refreshStatus(link.id).catch((e: unknown) => app.noteError(e))}
      >
        ⟳
      </Button>
    </div>
  {/if}

  <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs">
    <button
      type="button"
      class="inline-flex items-center gap-1 text-primary-fg hover:underline"
      onclick={() => api.openFolder(link.id)}
    >
      <FolderOpen size={13} aria-hidden="true" />{t('desktop.detail.open_folder')}
    </button>
    <button
      type="button"
      class="inline-flex items-center gap-1 text-primary-fg hover:underline"
      onclick={() => api.openHistory(link.id)}
    >
      <History size={13} aria-hidden="true" />{t('desktop.detail.history')}
    </button>
    <button
      type="button"
      class="inline-flex items-center gap-1 text-primary-fg hover:underline"
      onclick={() => showFolderSettings(link.id)}
    >
      <Settings size={13} aria-hidden="true" />{t('desktop.detail.settings')}
    </button>
  </div>
</div>
