<script lang="ts">
  // The files of one version (the current one by default), from its manifest. For a
  // conflict snapshot it shows what differs from the server's current version
  // instead: files added, removed and modified (the "Compare" of the conflict banner).
  import FolderIcon from '@lucide/svelte/icons/folder';
  import type { FolderInfo, Manifest, SnapshotInfo } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Input,
    Skeleton,
    Tag,
    formatBytes,
    formatDateTime,
    formatNumber
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { getManifest } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';
  import {
    diffManifests,
    filterEntries,
    mtimeOf,
    splitPath,
    type ManifestDiff
  } from '#lib/manifest.ts';
  import { tn } from '#lib/plural.ts';

  let {
    folder,
    inspect,
    tick,
    onreset
  }: {
    folder: FolderInfo;
    /** The version to show; `null` = the current one. */
    inspect: SnapshotInfo | null;
    tick: number;
    onreset: () => void;
  } = $props();

  const MAX_ROWS = 500;

  let manifest = $state<Manifest | null>(null);
  let headManifest = $state<Manifest | null>(null);
  let error = $state<string | null>(null);
  let query = $state('');

  const lang = $derived(getLang());
  const targetId = $derived(inspect?.id ?? folder.head_snapshot_id);
  const compare = $derived(inspect?.kind === 'conflict' && inspect.id !== folder.head_snapshot_id);
  const diff = $derived<ManifestDiff | null>(
    compare && manifest && headManifest ? diffManifests(headManifest, manifest) : null
  );
  const files = $derived(filterEntries(manifest?.entries ?? [], query));

  $effect(() => {
    void tick;
    const id = targetId;
    const headId = compare ? folder.head_snapshot_id : null;
    manifest = null;
    headManifest = null;
    error = null;
    if (!id) return;
    Promise.all([getManifest(id), headId ? getManifest(headId) : null])
      .then(([m, h]) => {
        if (id !== targetId) return;
        manifest = m;
        headManifest = h;
      })
      .catch((err) => {
        if (id === targetId) error = errorText(err);
      });
  });

  const modified = (d: Date) => formatDateTime(d, lang);
</script>

{#snippet fileRow(path: string, size: number, secs: Date | null, isDir: boolean)}
  {@const parts = splitPath(path)}
  <li class="flex items-baseline gap-2 border-b border-border py-1.5 last:border-b-0">
    {#if isDir}<FolderIcon
        size={13}
        class="flex-none translate-y-0.5 text-muted"
        aria-hidden="true"
      />{/if}
    <span class="min-w-0 flex-1 break-all">
      <span class="text-muted">{parts.dir}</span><span class="font-medium">{parts.name}</span>
      {#if isDir}<span class="text-xs text-muted"> · {t('files.empty_dir')}</span>{/if}
    </span>
    {#if !isDir}
      <span class="flex-none text-xs text-muted tabular-nums">{formatBytes(size, lang)}</span>
    {/if}
    {#if secs}<span class="hidden flex-none text-xs text-muted xl:inline">{modified(secs)}</span
      >{/if}
  </li>
{/snippet}

<div class="mb-3 flex flex-wrap items-center gap-2 text-xs text-muted">
  <span class="flex-1">
    {#if compare}
      {t('files.diff_title')}
    {:else if inspect}
      {t('files.title_version', { when: formatDateTime(inspect.created_at, lang) })}
    {:else}
      {t('files.title_head')}
    {/if}
  </span>
  {#if inspect}
    <Button size="sm" variant="ghost" onclick={onreset}>{t('files.show_head')}</Button>
  {/if}
</div>

{#if !targetId}
  <p class="py-6 text-center text-muted">{t('files.no_version')}</p>
{:else if error}
  <Banner variant="danger">{error}</Banner>
{:else if manifest === null || (compare && headManifest === null && folder.head_snapshot_id)}
  <div class="flex flex-col gap-2" aria-busy="true">
    <Skeleton class="h-5 w-full" />
    <Skeleton class="h-5 w-4/5" />
    <Skeleton class="h-5 w-3/5" />
  </div>
{:else if diff}
  <p class="mb-3 text-xs text-muted">{t('files.diff_hint')}</p>
  {#if diff.added.length + diff.removed.length + diff.modified.length === 0}
    <Banner variant="success">{t('files.diff_none')}</Banner>
  {:else}
    {#if diff.added.length}
      <h3 class="mb-1 flex items-center gap-2 text-sm">
        <Tag variant="success">{t('files.diff_added')}</Tag>
        {diff.added.length}
      </h3>
      <ul class="mb-3">
        {#each diff.added as entry (entry.path)}
          {@render fileRow(entry.path, entry.size, null, entry.kind === 'dir')}
        {/each}
      </ul>
    {/if}
    {#if diff.removed.length}
      <h3 class="mb-1 flex items-center gap-2 text-sm">
        <Tag variant="danger">{t('files.diff_removed')}</Tag>
        {diff.removed.length}
      </h3>
      <ul class="mb-3">
        {#each diff.removed as entry (entry.path)}
          {@render fileRow(entry.path, entry.size, null, entry.kind === 'dir')}
        {/each}
      </ul>
    {/if}
    {#if diff.modified.length}
      <h3 class="mb-1 flex items-center gap-2 text-sm">
        <Tag variant="warning">{t('files.diff_modified')}</Tag>
        {diff.modified.length}
      </h3>
      <ul class="mb-3">
        {#each diff.modified as change (change.path)}
          {@const parts = splitPath(change.path)}
          <li class="flex items-baseline gap-2 border-b border-border py-1.5 last:border-b-0">
            <span class="min-w-0 flex-1 break-all">
              <span class="text-muted">{parts.dir}</span><span class="font-medium"
                >{parts.name}</span
              >
            </span>
            <span class="flex-none text-xs text-muted tabular-nums">
              {formatBytes(change.before.size, lang)} → {formatBytes(change.after.size, lang)}
            </span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
  <p class="text-xs text-muted">
    {t('files.diff_unchanged', { count: formatNumber(diff.unchanged, lang) })}
  </p>
{:else if manifest.entries.length === 0}
  <p class="py-6 text-center text-muted">{t('files.empty')}</p>
{:else}
  <div class="mb-2 flex items-center gap-2">
    <Input
      type="search"
      bind:value={query}
      placeholder={t('files.search')}
      aria-label={t('files.search')}
    />
    <span class="flex-none text-xs whitespace-nowrap text-muted">
      {tn('files.count', files.length, { count: formatNumber(files.length, lang) })}
    </span>
  </div>
  <ul>
    {#each files.slice(0, MAX_ROWS) as entry (entry.path)}
      {@render fileRow(entry.path, entry.size, mtimeOf(entry), entry.kind === 'dir')}
    {/each}
  </ul>
  {#if files.length > MAX_ROWS}
    <p class="mt-2 text-xs text-muted">{t('files.truncated', { count: MAX_ROWS })}</p>
  {/if}
{/if}
