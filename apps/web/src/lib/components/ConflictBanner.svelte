<script lang="ts">
  // A folder with an unresolved conflict: a device saved a version different from the
  // server's. "Keep server" resolves it (the device's version stays in the history);
  // "Keep <device>" makes the device's version the head; "Compare" shows the file
  // differences. Only editors can decide; viewers see the notice.
  import type { FolderInfo, SnapshotInfo } from '@cloudit/api-types';
  import { Banner, Button, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { ApiError, listSnapshots, resolveSnapshot, restoreSnapshot } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';

  let {
    folder,
    canEdit,
    tick,
    oncompare,
    onchanged
  }: {
    folder: FolderInfo;
    canEdit: boolean;
    /** Changes whenever the data may be stale (live events). */
    tick: number;
    oncompare: (snapshot: SnapshotInfo) => void;
    onchanged: () => void;
  } = $props();

  let conflicts = $state<SnapshotInfo[]>([]);
  let busy = $state<'server' | 'device' | null>(null);

  $effect(() => {
    void tick;
    void folder.unresolved_conflicts;
    const id = folder.id;
    listSnapshots(id, { kind: 'conflict', unresolved: true, limit: 10 })
      .then((page) => {
        if (id === folder.id) conflicts = page.snapshots;
      })
      .catch(() => undefined);
  });

  const current = $derived(conflicts[0]);
  const device = $derived(current?.device_name ?? t('conflict.a_device'));

  async function keepServer(snapshot: SnapshotInfo) {
    busy = 'server';
    try {
      await resolveSnapshot(snapshot.id);
      toast.success(t('conflict.kept_server'));
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      busy = null;
      onchanged();
    }
  }

  async function keepDevice(snapshot: SnapshotInfo) {
    busy = 'device';
    try {
      await restoreSnapshot(snapshot.id, {
        expected_head: folder.head_snapshot_id ?? undefined
      });
      toast.success(t('conflict.kept_device', { device }));
    } catch (err) {
      // The head moved while the user was deciding: refresh and ask again.
      toast.error(
        err instanceof ApiError && err.code === 'conflict'
          ? t('history.head_moved')
          : errorText(err)
      );
    } finally {
      busy = null;
      onchanged();
    }
  }
</script>

{#if current}
  <Banner variant="warning">
    <b>{folder.name}</b>: {t('conflict.text', { device })}
    {#if conflicts.length > 1}
      <span class="text-muted">({t('conflict.more', { count: conflicts.length - 1 })})</span>
    {/if}
    <div class="mt-0.5 text-xs text-muted">{t('conflict.hint')}</div>
    {#snippet actions()}
      <Button size="sm" onclick={() => oncompare(current)}>{t('conflict.compare')}</Button>
      {#if canEdit}
        <Button
          size="sm"
          loading={busy === 'server'}
          disabled={busy !== null}
          onclick={() => keepServer(current)}
        >
          {t('conflict.keep_server')}
        </Button>
        <Button
          size="sm"
          loading={busy === 'device'}
          disabled={busy !== null}
          onclick={() => keepDevice(current)}
        >
          {t('conflict.keep_device', { device })}
        </Button>
      {/if}
    {/snippet}
  </Banner>
{/if}
