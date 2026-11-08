<script lang="ts" module>
  export type PanelTab = 'versions' | 'files' | 'settings';
</script>

<script lang="ts">
  // The selected folder: details, then the Versions / Files / Settings tabs. The
  // tab and the version shown in Files are owned by the page (the conflict banner
  // opens "Files" on its snapshot).
  import Folder from '@lucide/svelte/icons/folder';
  import type { FolderInfo, ScopeRole, SnapshotInfo } from '@cloudit/api-types';
  import { Tabs, formatBytes, formatNumber, formatRelative } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import FilesTab from './FilesTab.svelte';
  import FolderSettingsTab from './FolderSettingsTab.svelte';
  import HistoryTab from './HistoryTab.svelte';

  let {
    folder,
    role,
    tab = $bindable('versions'),
    inspect = $bindable(null),
    tick,
    onchanged,
    ontrashed
  }: {
    folder: FolderInfo;
    role: ScopeRole;
    tab?: PanelTab;
    inspect?: SnapshotInfo | null;
    tick: number;
    onchanged: () => void;
    ontrashed: () => void;
  } = $props();

  const lang = $derived(getLang());
  const limit = (n: number) => (n === 0 ? '∞' : formatNumber(n, lang));
  const synced = $derived(
    folder.head
      ? t('folder.synced_by', {
          when: formatRelative(folder.head.created_at, lang),
          who: folder.head.device_name ?? folder.head.author_name
        })
      : t('folder.never_synced')
  );
  const tabs = $derived([
    { value: 'versions' as const, label: t('tab.versions') },
    { value: 'files' as const, label: t('tab.files') },
    { value: 'settings' as const, label: t('tab.settings') }
  ]);

  function inspectSnapshot(snapshot: SnapshotInfo) {
    inspect = snapshot;
    tab = 'files';
  }
</script>

<section class="card p-[18px] lg:sticky lg:top-5" aria-label={folder.name}>
  <div class="flex items-center gap-2.5">
    <span class="icon-tile">
      {#if folder.emoji}{folder.emoji}{:else}<Folder
          size={18}
          class="text-muted"
          aria-hidden="true"
        />{/if}
    </span>
    <div class="min-w-0">
      <h2 class="truncate text-[17px]">{folder.name}</h2>
      <div class="text-xs text-muted">{synced}</div>
    </div>
  </div>

  <dl class="my-3.5 grid grid-cols-2 gap-2.5">
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <dt class="text-xs text-muted">{t('folder.size')}</dt>
      <dd class="text-[15px] font-bold">
        {folder.head ? formatBytes(folder.head.logical_size, lang) : '—'}
      </dd>
    </div>
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <dt class="text-xs text-muted">{t('folder.files')}</dt>
      <dd class="text-[15px] font-bold">
        {folder.head ? formatNumber(folder.head.file_count, lang) : '—'}
      </dd>
    </div>
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <dt class="text-xs text-muted">{t('folder.versions')}</dt>
      <dd class="text-[15px] font-bold">
        {formatNumber(folder.version_count, lang)} / {limit(
          folder.effective_retention_max_versions
        )}
      </dd>
    </div>
    <div class="rounded-lg bg-surface-alt px-2.5 py-2">
      <dt class="text-xs text-muted">{t('folder.per_day')}</dt>
      <dd class="text-[15px] font-bold">
        {folder.effective_retention_max_per_day === 0
          ? t('folder.unlimited')
          : t('folder.max', { count: formatNumber(folder.effective_retention_max_per_day, lang) })}
      </dd>
    </div>
  </dl>

  <Tabs bind:value={tab} items={tabs} label={t('tab.label')} />

  {#if tab === 'versions'}
    <HistoryTab {folder} {role} {tick} oninspect={inspectSnapshot} {onchanged} />
  {:else if tab === 'files'}
    <FilesTab {folder} {inspect} {tick} onreset={() => (inspect = null)} />
  {:else}
    <FolderSettingsTab {folder} {role} {onchanged} {ontrashed} />
  {/if}
</section>
