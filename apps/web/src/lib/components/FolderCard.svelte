<script lang="ts">
  // One folder of a scope: icon, name, last sync, status, numbers and who uploaded last.
  import Folder from '@lucide/svelte/icons/folder';
  import Laptop from '@lucide/svelte/icons/laptop';
  import type { FolderInfo } from '@cloudit/api-types';
  import { Tag, formatBytes, formatNumber, formatRelative, type TagVariant } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';

  let {
    folder,
    selected,
    onselect
  }: { folder: FolderInfo; selected: boolean; onselect: () => void } = $props();

  const lang = $derived(getLang());
  const status = $derived<{ label: string; variant: TagVariant }>(
    folder.unresolved_conflicts > 0
      ? { label: t('folder.status_conflict'), variant: 'warning' }
      : folder.head
        ? { label: t('folder.status_synced'), variant: 'success' }
        : { label: t('folder.status_empty'), variant: 'neutral' }
  );
  const uploader = $derived(
    folder.head
      ? `${folder.head.author_name || t('common.unknown')} · ${folder.head.device_name ?? t('common.web')}`
      : null
  );
</script>

<button
  type="button"
  class={[
    'card flex cursor-pointer flex-col gap-3 p-[18px] text-left transition-shadow',
    selected
      ? 'border-primary shadow-[0_0_0_3px_var(--cl-primary-soft)]'
      : 'hover:border-border-strong'
  ]}
  aria-pressed={selected}
  onclick={onselect}
>
  <span class="flex items-center gap-3">
    <span class="icon-tile">
      {#if folder.emoji}{folder.emoji}{:else}<Folder
          size={18}
          class="text-muted"
          aria-hidden="true"
        />{/if}
    </span>
    <span class="min-w-0 flex-1">
      <b class="block truncate">{folder.name}</b>
      <span class="block text-xs text-muted">
        {folder.head
          ? t('folder.synced_ago', { when: formatRelative(folder.head.created_at, lang) })
          : t('folder.never_synced')}
      </span>
    </span>
    <Tag variant={status.variant}>{status.label}</Tag>
  </span>
  <span class="flex gap-[18px] text-xs text-muted">
    <span>
      <b class="block text-sm text-text">
        {folder.head ? formatBytes(folder.head.logical_size, lang) : '—'}
      </b>
      {t('folder.size')}
    </span>
    <span>
      <b class="block text-sm text-text">
        {folder.head ? formatNumber(folder.head.file_count, lang) : '—'}
      </b>
      {t('folder.files')}
    </span>
    <span>
      <b class="block text-sm text-text">{formatNumber(folder.version_count, lang)}</b>
      {t('folder.versions')}
    </span>
  </span>
  {#if uploader}
    <span class="flex items-center gap-2 border-t border-border pt-2.5 text-xs text-muted">
      <Laptop size={14} class="flex-none" aria-hidden="true" />
      <span class="truncate">{t('folder.last_upload', { who: uploader })}</span>
    </span>
  {/if}
</button>
