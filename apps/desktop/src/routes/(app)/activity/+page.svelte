<script lang="ts">
  // What this device did: syncs, conflicts, links, sign-ins (the engine's activity log).
  import Clock from '@lucide/svelte/icons/clock';
  import type { ActivityEntry } from '@cloudit/api-types';
  import { Banner, EmptyState, Skeleton, Tag, formatBytes, formatDateTime } from '@cloudit/ui';
  import { getLang, t, type MessageKey } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import * as api from '#lib/api.ts';
  import { errorText, kindKey } from '#lib/errors.ts';
  import { tn } from '#lib/plural.ts';

  const ACTIONS: Record<string, MessageKey> = {
    push: 'desktop.activity.action_push',
    pull: 'desktop.activity.action_pull',
    conflict: 'desktop.activity.action_conflict',
    keep_local: 'desktop.activity.action_keep_local',
    keep_server: 'desktop.activity.action_keep_server',
    decide_later: 'desktop.activity.action_decide_later',
    sync: 'desktop.activity.action_sync',
    link: 'desktop.activity.action_link',
    unlink: 'desktop.activity.action_unlink',
    login: 'desktop.activity.action_login',
    logout: 'desktop.activity.action_logout'
  };

  let entries = $state<ActivityEntry[] | null>(null);
  let error = $state<string | null>(null);
  const lang = $derived(getLang());

  async function load() {
    try {
      entries = await api.activity(null, 200);
      error = null;
    } catch (err) {
      app.noteError(err);
      error = errorText(err);
    }
  }

  // A finished operation adds a row.
  $effect(() => {
    void app.folders;
    void load();
  });

  const actionText = (entry: ActivityEntry) =>
    t(ACTIONS[entry.action] ?? 'desktop.activity.action_other');

  function errorKindText(entry: ActivityEntry): string {
    const kind = entry.error_kind;
    if (!kind) return t('desktop.activity.error');
    // The kind is one of the engine's stable names; an unknown one falls back.
    const key = kindKey(kind as Parameters<typeof kindKey>[0]);
    return key ? t(key) : t('desktop.activity.error');
  }
</script>

<div class="flex h-full flex-col gap-3">
  <h1 class="!text-[19px]">{t('desktop.activity.title')}</h1>

  {#if error}
    <Banner variant="danger">{error}</Banner>
  {:else if entries === null}
    <Skeleton class="h-40 w-full" />
  {:else if entries.length === 0}
    <EmptyState icon={Clock} title={t('desktop.activity.empty')} />
  {:else}
    <div class="card overflow-auto">
      <table class="data-table text-[13px]">
        <thead>
          <tr>
            <th>{t('desktop.activity.when')}</th>
            <th>{t('desktop.activity.folder')}</th>
            <th>{t('desktop.activity.what')}</th>
            <th>{t('desktop.activity.result')}</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as entry (entry.id)}
            <tr>
              <td class="whitespace-nowrap text-muted">{formatDateTime(entry.at, lang)}</td>
              <td>{entry.folder_name ?? '–'}</td>
              <td>
                {actionText(entry)}
                {#if entry.files > 0 || entry.bytes > 0}
                  <span class="block text-xs text-muted">
                    {tn('desktop.activity.counts', entry.files, {
                      bytes: formatBytes(entry.bytes, lang)
                    })}
                  </span>
                {/if}
              </td>
              <td>
                {#if entry.outcome === 'ok'}
                  <Tag variant="success">{t('desktop.activity.ok')}</Tag>
                {:else if entry.outcome === 'error'}
                  <Tag variant="danger">{t('desktop.activity.error')}</Tag>
                  <span class="mt-0.5 block text-xs text-muted">{errorKindText(entry)}</span>
                {:else}
                  <Tag variant="neutral">{t('desktop.activity.skipped')}</Tag>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>
