<script lang="ts">
  // Both this computer and the server changed: show each side's summary and the files,
  // and let the user keep one version (the other stays in the server history).
  import Laptop from '@lucide/svelte/icons/laptop';
  import Cloud from '@lucide/svelte/icons/cloud';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import { page } from '$app/state';
  import { Banner, Button, Spinner, Tag, confirmAction, toast, type TagVariant } from '@cloudit/ui';
  import { t, type MessageKey } from '@cloudit/ui/i18n';
  import { reportError, reportOutcome, showFolders } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import OpProgress from '#lib/components/OpProgress.svelte';
  import { tn } from '#lib/plural.ts';
  import {
    changeCounts,
    displayState,
    fileRows,
    type ChangeCounts,
    type FileRow
  } from '#lib/status.ts';
  import type { PathChangeKind } from '@cloudit/api-types';

  const id = $derived(Number(page.params.id));
  const link = $derived(app.folders.find((f) => f.id === id));
  const status = $derived(app.statuses[id]);
  const folderState = $derived(link ? displayState(link, status) : 'unknown');
  const busy = $derived(app.busy[id] !== undefined);
  let deciding = $state(false);
  let preparing = $state(false);
  let prepared = $state(false);

  // The status may not be loaded yet when the screen opens from a notification.
  $effect(() => {
    if (link && !status && app.busy[id] === undefined) {
      void api.refreshStatus(id).catch((error: unknown) => app.noteError(error));
    }
  });

  // A decision needs this computer's version stored on the server as a conflict
  // snapshot: a sync does that (nothing local is touched) and reports the decision due.
  $effect(() => {
    if (!link || !status || prepared || preparing || app.busy[id] !== undefined) return;
    if (status.state !== 'conflict' || status.pending_conflict_id) return;
    preparing = true;
    prepared = true;
    api
      .syncFolder(id)
      .catch((error: unknown) => reportError(error))
      .finally(() => (preparing = false));
  });

  const ready = $derived(!!status?.pending_conflict_id && !preparing);
  const local = $derived<ChangeCounts>(changeCounts(status?.local_changes ?? []));
  const remote = $derived<ChangeCounts>(changeCounts(status?.remote_changes ?? []));
  const rows = $derived(status ? fileRows(status) : []);
  const isConflict = $derived(folderState === 'conflict');

  const CHANGE_LABEL: Record<PathChangeKind, MessageKey> = {
    added: 'desktop.conflict.file_added',
    modified: 'desktop.conflict.file_modified',
    removed: 'desktop.conflict.file_removed'
  };
  const CHANGE_TAG: Record<PathChangeKind, TagVariant> = {
    added: 'primary',
    modified: 'warning',
    removed: 'danger'
  };

  async function decide(kind: 'local' | 'server') {
    if (!link) return;
    if (kind === 'server') {
      const ok = await confirmAction({
        title: t('desktop.conflict.keep_server'),
        message: t('desktop.conflict.keep_server_help'),
        confirmLabel: t('desktop.conflict.keep_server')
      });
      if (!ok) return;
    }
    deciding = true;
    try {
      const report = await (kind === 'local' ? api.keepLocal(id) : api.keepServer(id));
      reportOutcome(link, report);
      if (report.outcome.outcome !== 'needs_decision') await showFolders();
    } catch (error) {
      reportError(error);
    } finally {
      deciding = false;
    }
  }

  async function later() {
    try {
      await api.decideLater(id);
      await app.loadFolders();
      toast.info(t('desktop.conflict.paused'));
      await showFolders();
    } catch (error) {
      reportError(error);
    }
  }
</script>

{#snippet counts(c: ChangeCounts, both: number)}
  <div class="mt-2 space-y-1 text-xs">
    {#if c.added + c.modified + c.removed === 0}
      <div class="text-muted">{t('desktop.conflict.nothing')}</div>
    {/if}
    {#if c.added > 0}
      <div><Tag variant="primary">+{c.added}</Tag> {tn('desktop.conflict.added', c.added)}</div>
    {/if}
    {#if c.modified > 0}
      <div>
        <Tag variant="warning">~{c.modified}</Tag>
        {tn('desktop.conflict.modified', c.modified)}
      </div>
    {/if}
    {#if c.removed > 0}
      <div>
        <Tag variant="danger">−{c.removed}</Tag>
        {tn('desktop.conflict.removed', c.removed)}
      </div>
    {/if}
    {#if both > 0}
      <div class="text-muted">{tn('desktop.conflict.both', both)}</div>
    {/if}
  </div>
{/snippet}

{#snippet change(kind: FileRow['local'])}
  {#if kind}<Tag variant={CHANGE_TAG[kind]}>{t(CHANGE_LABEL[kind])}</Tag>{/if}
{/snippet}

<div class="mx-auto max-w-[720px]">
  {#if !link}
    <Banner variant="info">{t('desktop.error.not_linked')}</Banner>
    <div class="mt-3"><Button onclick={showFolders}>{t('desktop.conflict.back')}</Button></div>
  {:else if !isConflict && folderState === 'server_conflict'}
    <div class="card p-[18px]">
      <h2 class="flex items-center gap-2">
        <TriangleAlert size={18} class="text-warning" aria-hidden="true" />
        {t('desktop.conflict.on_server_title')}
      </h2>
      <p class="mt-2 text-muted">{t('desktop.conflict.on_server_body')}</p>
      <div class="mt-4 flex gap-2">
        <Button variant="primary" onclick={() => api.openHistory(id)}>
          {t('desktop.detail.history')}
        </Button>
        <Button onclick={showFolders}>{t('desktop.conflict.back')}</Button>
      </div>
    </div>
  {:else if !isConflict && status}
    <div class="card p-[18px]">
      <p class="text-muted">{t('desktop.conflict.none')}</p>
      <div class="mt-4"><Button onclick={showFolders}>{t('desktop.conflict.back')}</Button></div>
    </div>
  {:else}
    <div class="card p-[18px]">
      <h2 class="flex items-center gap-2">
        <TriangleAlert size={18} class="text-warning" aria-hidden="true" />
        {t('desktop.conflict.title', { name: link.folder_name })}
      </h2>
      <p class="mt-1 text-muted">{t('desktop.conflict.body')}</p>

      {#if !status}
        <div class="mt-4 flex items-center gap-2 text-muted">
          <Spinner size={14} />{t('desktop.folders.checking')}
        </div>
      {:else}
        <div class="my-3.5 grid grid-cols-2 gap-3">
          <div class="card p-3">
            <b class="flex items-center gap-1.5"
              ><Laptop size={15} aria-hidden="true" />{t('desktop.conflict.this_pc')}</b
            >
            {@render counts(local, status.conflicting.length)}
            <Button
              variant="primary"
              block
              class="mt-2.5"
              disabled={!ready || busy || deciding}
              onclick={() => decide('local')}
            >
              {t('desktop.conflict.keep_local')}
            </Button>
            <p class="hint">{t('desktop.conflict.keep_local_help')}</p>
          </div>
          <div class="card p-3">
            <b class="flex items-center gap-1.5"
              ><Cloud size={15} aria-hidden="true" />{t('desktop.conflict.server')}</b
            >
            {@render counts(remote, status.conflicting.length)}
            <Button
              block
              class="mt-2.5"
              disabled={!ready || busy || deciding}
              onclick={() => decide('server')}
            >
              {t('desktop.conflict.keep_server')}
            </Button>
            <p class="hint">{t('desktop.conflict.keep_server_help')}</p>
          </div>
        </div>

        {#if busy || deciding || preparing}
          <div class="mb-3">
            <p class="mb-1 text-xs text-muted">
              {preparing ? t('desktop.conflict.preparing') : t('desktop.conflict.working')}
            </p>
            <OpProgress progress={app.progress[id]} />
          </div>
        {/if}

        <details open class="mb-3">
          <summary class="cursor-pointer text-xs text-muted"
            >{t('desktop.conflict.files')} ({rows.length})</summary
          >
          <div class="mt-2 max-h-44 overflow-auto rounded-lg border border-border">
            <table class="data-table text-xs">
              <thead>
                <tr>
                  <th>&nbsp;</th>
                  <th class="w-24">{t('desktop.conflict.file_here')}</th>
                  <th class="w-24">{t('desktop.conflict.file_there')}</th>
                </tr>
              </thead>
              <tbody>
                {#each rows as row (row.path)}
                  <tr>
                    <td class="!py-1.5">
                      <span class="font-mono text-[11.5px] break-all">{row.path}</span>
                      {#if row.both}<Tag variant="warning" class="ms-1"
                          >{t('desktop.conflict.file_both')}</Tag
                        >{/if}
                    </td>
                    <td class="!py-1.5">{@render change(row.local)}</td>
                    <td class="!py-1.5">{@render change(row.remote)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </details>

        <Banner variant="info">{t('desktop.conflict.later_edit')}</Banner>
      {/if}

      <div class="mt-3.5 flex items-center gap-2">
        <Button variant="ghost" disabled={!ready || busy || deciding} onclick={later}
          >{t('desktop.conflict.later')}</Button
        >
        <Button class="ms-auto" onclick={showFolders}>{t('desktop.conflict.back')}</Button>
      </div>
    </div>
  {/if}
</div>
