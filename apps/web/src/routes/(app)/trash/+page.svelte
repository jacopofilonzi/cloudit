<script lang="ts">
  // Trashed scopes and folders: restore (asking for a new name if the old one was
  // taken), delete for good, or empty everything. Items leave the list at once;
  // the space is freed when the server's purge job has run.
  import { onMount } from 'svelte';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import type { TrashItem } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    EmptyState,
    Skeleton,
    Table,
    TableActions,
    Tag,
    confirmAction,
    formatDate,
    formatRelative,
    toast
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import {
    ApiError,
    emptyTrash,
    listMembers,
    listTrash,
    purgeTrashedFolder,
    purgeTrashedScope,
    restoreFolder,
    restoreScope
  } from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import PromptModal from '#lib/components/PromptModal.svelte';
  import { errorText } from '#lib/errors.ts';
  import { session } from '#lib/session.svelte.ts';

  let items = $state<TrashItem[] | null>(null);
  let names = $state<Record<string, string>>({});
  let error = $state<string | null>(null);
  let renaming = $state<TrashItem | null>(null);
  let renameOpen = $state(false);
  let busy = $state(false);

  const lang = $derived(getLang());

  async function load() {
    try {
      const list = await listTrash();
      items = list;
      error = null;
      await loadNames(list);
    } catch (err) {
      error = errorText(err);
    }
  }

  /** Who deleted a folder: the members of its scope know the names. */
  async function loadNames(list: TrashItem[]) {
    const scopes = [...new Set(list.filter((i) => i.kind === 'folder').map((i) => i.scope_id))];
    const found: Record<string, string> = {};
    await Promise.all(
      scopes.map(async (id) => {
        try {
          for (const member of await listMembers(id))
            found[member.user.id] = member.user.display_name;
        } catch {
          // The names are a nicety.
        }
      })
    );
    names = found;
  }

  onMount(() => void load());

  function deletedBy(item: TrashItem): string | null {
    if (item.kind === 'scope' || item.deleted_by === session.user?.id) return t('common.you');
    return item.deleted_by ? (names[item.deleted_by] ?? null) : null;
  }

  async function doRestore(item: TrashItem, name?: string) {
    const body = name ? { name } : {};
    if (item.kind === 'scope') await restoreScope(item.id, body);
    else await restoreFolder(item.id, body);
    toast.success(t('trash.restored', { name: name ?? item.name }));
    void app.loadScopes();
    void app.loadUsage();
    await load();
  }

  async function restore(item: TrashItem) {
    try {
      await doRestore(item);
    } catch (err) {
      if (err instanceof ApiError && err.code === 'name_taken') {
        renaming = item;
        renameOpen = true;
      } else {
        toast.error(errorText(err));
      }
    }
  }

  async function purge(item: TrashItem) {
    const ok = await confirmAction({
      title: t('trash.delete_title', { name: item.name }),
      message: item.kind === 'scope' ? t('trash.delete_scope_body') : t('trash.delete_folder_body'),
      confirmLabel: t('trash.delete_confirm'),
      danger: true
    });
    if (!ok) return;
    try {
      if (item.kind === 'scope') await purgeTrashedScope(item.id);
      else await purgeTrashedFolder(item.id);
      toast.success(t('trash.delete_queued', { name: item.name }));
    } catch (err) {
      toast.error(errorText(err));
    }
    await load();
  }

  async function empty() {
    const ok = await confirmAction({
      title: t('trash.empty_title'),
      message: t('trash.empty_body'),
      confirmLabel: t('trash.empty'),
      danger: true
    });
    if (!ok) return;
    busy = true;
    try {
      await emptyTrash();
      toast.success(t('trash.emptied'));
    } catch (err) {
      toast.error(errorText(err));
    } finally {
      busy = false;
    }
    await load();
  }
</script>

<svelte:head><title>{t('trash.title')} · {t('app.name')}</title></svelte:head>

<div class="mb-[18px] flex flex-wrap items-center gap-3.5">
  <div class="min-w-0 flex-1">
    <h1>{t('trash.title')}</h1>
    <p class="text-muted">{t('trash.subtitle')}</p>
  </div>
  {#if items && items.length > 0}
    <Button variant="danger" loading={busy} onclick={empty}>{t('trash.empty')}</Button>
  {/if}
</div>

{#if error}
  <Banner variant="danger">{error}</Banner>
{:else if items === null}
  <Skeleton class="h-40 w-full" />
{:else if items.length === 0}
  <EmptyState icon={Trash2} title={t('trash.none_title')} description={t('trash.none_body')} />
{:else}
  <Table caption={t('trash.title')}>
    <thead>
      <tr>
        <th>{t('trash.col_item')}</th>
        <th>{t('trash.col_deleted')}</th>
        <th>{t('trash.col_purge')}</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each items as item (item.id)}
        {@const who = deletedBy(item)}
        <tr>
          <td>
            <div class="flex items-center gap-2">
              <b>{item.name}</b>
              <Tag variant="neutral">
                {item.kind === 'scope' ? t('trash.kind_scope') : t('trash.kind_folder')}
              </Tag>
            </div>
            {#if item.kind === 'folder'}
              <div class="text-xs text-muted">
                {t('trash.in_scope', { scope: item.scope_name })}
              </div>
            {/if}
          </td>
          <td class="text-muted">
            {formatRelative(item.deleted_at, lang)}{who
              ? ` · ${t('trash.deleted_by', { who })}`
              : ''}
          </td>
          <td class="text-muted">{formatDate(item.purge_at, lang)}</td>
          <td>
            <TableActions>
              <Button size="sm" onclick={() => restore(item)}>{t('trash.restore')}</Button>
              <Button size="sm" variant="danger" onclick={() => purge(item)}>
                {t('trash.delete_now')}
              </Button>
            </TableActions>
          </td>
        </tr>
      {/each}
    </tbody>
  </Table>
  <p class="note mt-4">{t('trash.space_note')}</p>
{/if}

<PromptModal
  bind:open={renameOpen}
  title={t('trash.rename_title')}
  description={renaming ? t('trash.rename_desc', { name: renaming.name }) : undefined}
  label={t('field.name')}
  value={renaming?.name ?? ''}
  submitLabel={t('trash.restore')}
  onsubmit={async (name) => {
    if (renaming) await doRestore(renaming, name);
  }}
/>
