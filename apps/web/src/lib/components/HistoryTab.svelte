<script lang="ts">
  // The versions of a folder, newest first, grouped by UTC day (the retention day).
  // Hover (or focus) a version for its actions: restore, ZIP, pin and label, and the
  // list of its files. Viewers only get the read actions.
  import { untrack } from 'svelte';
  import FileArchive from '@lucide/svelte/icons/file-archive';
  import Files from '@lucide/svelte/icons/files';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
  import Tag from '@lucide/svelte/icons/tag';
  import type { FolderInfo, ScopeRole, SnapshotInfo } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    Skeleton,
    Tag as TagBadge,
    confirmAction,
    formatBytes,
    formatDateTime,
    formatTime,
    formatUtcDay,
    groupByUtcDay,
    toast
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import {
    ApiError,
    archiveUrl,
    listSnapshots,
    restoreSnapshot,
    updateSnapshot
  } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';
  import { atLeast } from '#lib/roles.ts';
  import PromptModal from './PromptModal.svelte';

  let {
    folder,
    role,
    tick,
    oninspect,
    onchanged
  }: {
    folder: FolderInfo;
    role: ScopeRole;
    /** Changes whenever the history may be stale (live events). */
    tick: number;
    oninspect: (snapshot: SnapshotInfo) => void;
    onchanged: () => void;
  } = $props();

  const PAGE = 30;
  const MAX_PAGE = 200;

  let items = $state<SnapshotInfo[] | null>(null);
  let cursor = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let labelling = $state<SnapshotInfo | null>(null);
  let labelOpen = $state(false);

  const lang = $derived(getLang());
  const canEdit = $derived(atLeast(role, 'editor'));
  const days = $derived(groupByUtcDay(items ?? [], (s) => s.created_at));

  async function load(id: string): Promise<void> {
    // Keep as many versions as the user had loaded, so a live refresh doesn't fold the list.
    const same = items?.[0]?.folder_id === id;
    const limit = same ? Math.min(MAX_PAGE, Math.max(PAGE, items?.length ?? 0)) : PAGE;
    if (!same) items = null;
    try {
      const page = await listSnapshots(id, { limit });
      if (id !== folder.id) return;
      items = page.snapshots;
      cursor = page.next_cursor;
      error = null;
    } catch (err) {
      if (id === folder.id) error = errorText(err);
    }
  }

  $effect(() => {
    const id = folder.id;
    void tick;
    void folder.head_snapshot_id;
    void folder.unresolved_conflicts;
    untrack(() => void load(id));
  });

  async function loadMore() {
    if (!cursor) return;
    loadingMore = true;
    try {
      const page = await listSnapshots(folder.id, { cursor, limit: PAGE });
      items = [...(items ?? []), ...page.snapshots];
      cursor = page.next_cursor;
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      loadingMore = false;
    }
  }

  function replace(updated: SnapshotInfo) {
    items = (items ?? []).map((s) => (s.id === updated.id ? updated : s));
  }

  async function restore(snapshot: SnapshotInfo) {
    const ok = await confirmAction({
      title: t('history.restore_title'),
      message: t('history.restore_body', { when: formatDateTime(snapshot.created_at, lang) }),
      confirmLabel: t('history.restore')
    });
    if (!ok) return;
    try {
      await restoreSnapshot(snapshot.id, { expected_head: folder.head_snapshot_id ?? undefined });
      toast.success(t('history.restored'));
    } catch (err) {
      toast.error(
        err instanceof ApiError && err.code === 'conflict'
          ? t('history.head_moved')
          : errorText(err)
      );
    } finally {
      onchanged();
    }
  }

  async function togglePin(snapshot: SnapshotInfo) {
    try {
      const updated = await updateSnapshot(snapshot.id, { pinned: !snapshot.pinned });
      replace(updated);
      toast.success(updated.pinned ? t('history.pinned') : t('history.unpinned'));
    } catch (err) {
      toast.error(errorText(err));
    }
  }

  function editLabel(snapshot: SnapshotInfo) {
    labelling = snapshot;
    labelOpen = true;
  }

  async function saveLabel(text: string) {
    if (!labelling) return;
    const updated = await updateSnapshot(labelling.id, { label: text === '' ? null : text });
    replace(updated);
    toast.success(t('common.saved'));
  }

  const dot = (s: SnapshotInfo) =>
    s.kind === 'conflict' && !s.conflict_resolved_at
      ? 'before:border-warning before:bg-warning'
      : s.is_head
        ? 'before:border-primary before:bg-primary'
        : 'before:border-primary before:bg-surface';

  const who = (s: SnapshotInfo) =>
    `${s.author_name || t('common.unknown')} · ${s.device_name ?? t('common.web')}`;
</script>

{#if error}
  <Banner variant="danger">{error}</Banner>
{:else if items === null}
  <div class="flex flex-col gap-3" aria-busy="true">
    <Skeleton class="h-4 w-20" />
    <Skeleton class="h-12 w-full" />
    <Skeleton class="h-12 w-full" />
  </div>
{:else if items.length === 0}
  <p class="py-6 text-center text-muted">{t('history.empty')}</p>
{:else}
  {#each days as group (group.day)}
    <div class="mt-3.5 mb-0.5 text-[11px] tracking-[.08em] text-muted uppercase">
      {formatUtcDay(group.day, lang)}
    </div>
    <ul
      class="relative pl-[18px] before:absolute before:top-1.5 before:bottom-1.5 before:left-[5px] before:w-0.5 before:bg-border before:content-['']"
    >
      {#each group.items as s (s.id)}
        <li
          class={[
            "group relative pt-2 pb-3 before:absolute before:top-[13px] before:-left-[17px] before:size-2.5 before:rounded-full before:border-2 before:content-['']",
            dot(s)
          ]}
        >
          <div class="flex flex-wrap items-center gap-1.5 font-semibold">
            {formatTime(s.created_at, lang)}
            {#if s.is_head}<TagBadge>{t('history.tag_current')}</TagBadge>{/if}
            {#if s.kind === 'conflict'}
              <TagBadge variant={s.conflict_resolved_at ? 'neutral' : 'warning'}>
                {s.conflict_resolved_at ? t('history.tag_resolved') : t('history.tag_conflict')}
              </TagBadge>
            {/if}
            {#if s.kind === 'restore'}<TagBadge variant="neutral"
                >{t('history.tag_restore')}</TagBadge
              >{/if}
            {#if s.pinned}
              <TagBadge variant="neutral">
                <Pin size={11} aria-hidden="true" />
                {s.label ?? t('history.tag_pinned')}
              </TagBadge>
            {:else if s.label}
              <TagBadge variant="neutral">
                <Tag size={11} aria-hidden="true" />
                {s.label}
              </TagBadge>
            {/if}
          </div>
          <div class="mt-0.5 text-xs text-muted">
            {who(s)} · {formatBytes(s.logical_size, lang)}
          </div>
          <div
            class="mt-1.5 hidden flex-wrap gap-1.5 group-focus-within:flex group-hover:flex pointer-coarse:flex"
            role="group"
            aria-label={formatDateTime(s.created_at, lang)}
          >
            {#if canEdit && !s.is_head}
              <Button size="sm" onclick={() => restore(s)}>
                <RotateCcw size={13} aria-hidden="true" />
                {t('history.restore')}
              </Button>
            {/if}
            <Button size="sm" onclick={() => oninspect(s)}>
              <Files size={13} aria-hidden="true" />
              {t('history.files')}
            </Button>
            <Button size="sm" href={archiveUrl(s.id)} download>
              <FileArchive size={13} aria-hidden="true" />
              {t('history.zip')}
            </Button>
            {#if canEdit}
              <Button size="sm" onclick={() => togglePin(s)}>
                {#if s.pinned}
                  <PinOff size={13} aria-hidden="true" />
                  {t('history.unpin')}
                {:else}
                  <Pin size={13} aria-hidden="true" />
                  {t('history.pin')}
                {/if}
              </Button>
              <Button size="sm" onclick={() => editLabel(s)}>
                <Tag size={13} aria-hidden="true" />
                {t('history.label')}
              </Button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/each}
  {#if cursor}
    <div class="mt-1.5">
      <Button variant="ghost" size="sm" loading={loadingMore} onclick={loadMore}>
        {t('history.load_older')}
      </Button>
    </div>
  {/if}
{/if}

<PromptModal
  bind:open={labelOpen}
  title={t('history.label_title')}
  label={t('history.label_field')}
  hint={t('history.label_hint')}
  value={labelling?.label ?? ''}
  submitLabel={t('common.save')}
  allowEmpty
  onsubmit={saveLabel}
/>
