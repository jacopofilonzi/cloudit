<script lang="ts">
  // One linked folder in the list: icon, name, collection and last sync, status tag.
  import type { LinkedFolder } from '@cloudit/api-types';
  import { Spinner, formatRelative } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import { displayState } from '#lib/status.ts';
  import FolderIcon from './FolderIcon.svelte';
  import StatusTag from './StatusTag.svelte';

  let {
    link,
    selected,
    onselect
  }: { link: LinkedFolder; selected: boolean; onselect: () => void } = $props();

  const folderState = $derived(displayState(link, app.statuses[link.id]));
  const busy = $derived(app.busy[link.id] !== undefined);
  const when = $derived(
    link.last_sync_at
      ? t('desktop.folders.last_sync', { when: formatRelative(link.last_sync_at, getLang()) })
      : t('desktop.folders.never_synced')
  );
</script>

<button
  type="button"
  class={[
    'flex w-full items-center gap-3 border-b border-border px-3.5 py-[11px] text-left last:border-b-0',
    selected ? 'bg-primary-soft' : 'hover:bg-surface-alt'
  ]}
  aria-pressed={selected}
  onclick={onselect}
>
  <FolderIcon emoji={link.emoji} />
  <span class="min-w-0 flex-1">
    <b class="block truncate">{link.folder_name}</b>
    <span class="block text-xs text-muted">{link.scope_name} · {when}</span>
  </span>
  {#if busy}
    <Spinner size={14} />
  {/if}
  <StatusTag state={folderState} />
</button>
