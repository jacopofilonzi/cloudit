<script lang="ts">
  // A scope: header with members and actions, conflict banners, the folder cards and
  // the panel of the selected folder (`?folder=<id>`). Kept fresh by live events.
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Plus from '@lucide/svelte/icons/plus';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import LogOut from '@lucide/svelte/icons/log-out';
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import type { FolderInfo, ScopeMember, SnapshotInfo } from '@cloudit/api-types';
  import {
    AvatarGroup,
    Banner,
    Button,
    EmptyState,
    Menu,
    Skeleton,
    confirmAction,
    formatBytes,
    toast
  } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { listFolders, listMembers, removeMember, trashScope, updateScope } from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { coalesced } from '#lib/coalesce.ts';
  import ConflictBanner from '#lib/components/ConflictBanner.svelte';
  import FolderCard from '#lib/components/FolderCard.svelte';
  import FolderCreateModal from '#lib/components/FolderCreateModal.svelte';
  import FolderPanel, { type PanelTab } from '#lib/components/FolderPanel.svelte';
  import PromptModal from '#lib/components/PromptModal.svelte';
  import ShareModal from '#lib/components/ShareModal.svelte';
  import { errorText } from '#lib/errors.ts';
  import { onLive } from '#lib/live.svelte.ts';
  import { tn } from '#lib/plural.ts';
  import { atLeast } from '#lib/roles.ts';
  import { joinNames } from '#lib/scope-text.ts';
  import { session } from '#lib/session.svelte.ts';

  const LAST_SCOPE_KEY = 'cloudit.lastScope';

  const scopeId = $derived(page.params.id ?? '');
  const scope = $derived(app.scope(scopeId));
  const lang = $derived(getLang());
  const role = $derived(scope?.role ?? 'viewer');
  const canEdit = $derived(atLeast(role, 'editor'));

  let folders = $state<FolderInfo[] | null>(null);
  let members = $state<ScopeMember[] | null>(null);
  let loadError = $state<string | null>(null);
  /** Bumped by live events: panels reload when it changes. */
  let tick = $state(0);

  let tab = $state<PanelTab>('versions');
  let inspect = $state<SnapshotInfo | null>(null);
  let sharing = $state(false);
  let creating = $state(false);
  let renaming = $state(false);

  const selectedId = $derived.by(() => {
    const wanted = page.url.searchParams.get('folder');
    return folders?.find((f) => f.id === wanted)?.id ?? folders?.[0]?.id;
  });
  const selected = $derived(folders?.find((f) => f.id === selectedId));
  const totalSize = $derived(
    (folders ?? []).reduce((sum, folder) => sum + (folder.head?.logical_size ?? 0), 0)
  );
  const conflicted = $derived((folders ?? []).filter((f) => f.unresolved_conflicts > 0));
  const others = $derived((members ?? []).filter((m) => m.role !== 'owner'));

  async function loadFolders(id: string): Promise<void> {
    try {
      const list = await listFolders(id);
      if (id === scopeId) {
        folders = list;
        loadError = null;
      }
    } catch (err) {
      if (id === scopeId) loadError = errorText(err);
    }
  }

  async function loadMembers(id: string): Promise<void> {
    try {
      const list = await listMembers(id);
      if (id === scopeId) members = list;
    } catch {
      // Not a member any more: the scope list refresh reports it.
    }
  }

  const refreshFolders = coalesced(async () => {
    await loadFolders(scopeId);
    tick += 1;
  });
  const refreshMembers = coalesced(async () => {
    await loadMembers(scopeId);
    await app.loadScopes();
  });

  // Load the data of the scope in the URL; remember it as the one to open at `/`.
  $effect(() => {
    const id = scopeId;
    folders = null;
    members = null;
    loadError = null;
    tab = 'versions';
    inspect = null;
    void loadFolders(id);
    void loadMembers(id);
    try {
      localStorage.setItem(LAST_SCOPE_KEY, id);
    } catch {
      // Not remembered; `/` opens the first scope.
    }
  });

  // Another folder: back to its versions (unless "Compare" just selected it).
  let keepTab = false;
  $effect(() => {
    void selectedId;
    if (keepTab) {
      keepTab = false;
      return;
    }
    tab = 'versions';
    inspect = null;
  });

  $effect(() =>
    onLive((event) => {
      if (event.type === 'resync') {
        refreshFolders();
        refreshMembers();
        return;
      }
      if (event.type === 'usage_changed' || event.type === 'hello') return;
      if (event.data.scope_id !== scopeId) return;
      if (event.type === 'membership_changed') refreshMembers();
      else if (event.type === 'scope_changed') {
        if (event.data.change === 'trashed' || event.data.change === 'deleted') {
          if (scope?.role !== 'owner' || event.data.change === 'deleted') gone();
        } else {
          refreshMembers();
        }
      } else refreshFolders();
    })
  );

  function gone() {
    toast.info(t('live.scope_gone'));
    void app.loadScopes();
    void goto(resolve('/(app)'));
  }

  function select(folder: FolderInfo) {
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve() plus a query
    void goto(`${resolve('/(app)/scopes/[id]', { id: scopeId })}?folder=${folder.id}`, {
      replace: true,
      reset: false
    });
  }

  function compare(snapshot: SnapshotInfo) {
    const folder = folders?.find((f) => f.id === snapshot.folder_id);
    if (folder && folder.id !== selectedId) {
      keepTab = true;
      select(folder);
    }
    inspect = snapshot;
    tab = 'files';
  }

  function created(folder: FolderInfo) {
    folders = [...(folders ?? []), folder].sort((a, b) =>
      a.name.localeCompare(b.name, lang, { sensitivity: 'base' })
    );
    void app.loadScopes();
    select(folder);
  }

  async function rename(name: string) {
    await updateScope(scopeId, { name });
    toast.success(t('scope.renamed'));
    await app.loadScopes();
  }

  async function trash() {
    if (!scope) return;
    const ok = await confirmAction({
      title: t('scope.trash_title', { name: scope.name }),
      message: t('scope.trash_body'),
      confirmLabel: t('scope.trash'),
      danger: true
    });
    if (!ok) return;
    try {
      await trashScope(scopeId);
      toast.success(t('scope.trashed'));
      await app.loadScopes();
      await goto(resolve('/(app)'));
    } catch (err) {
      toast.error(errorText(err));
    }
  }

  async function leave() {
    if (!scope || !session.user) return;
    const ok = await confirmAction({
      title: t('scope.leave_title', { name: scope.name }),
      message: t('scope.leave_body'),
      confirmLabel: t('scope.leave'),
      danger: true
    });
    if (!ok) return;
    try {
      await removeMember(scopeId, session.user.id);
      toast.success(t('scope.left'));
      await app.loadScopes();
      await goto(resolve('/(app)'));
    } catch (err) {
      toast.error(errorText(err));
    }
  }

  const menu = $derived(
    role === 'owner'
      ? [
          { label: t('scope.rename'), icon: Pencil, onclick: () => (renaming = true) },
          { label: t('scope.trash'), icon: Trash2, danger: true, onclick: trash }
        ]
      : [{ label: t('scope.leave'), icon: LogOut, danger: true, onclick: leave }]
  );

  const subtitle = $derived.by(() => {
    if (!scope || !folders) return '';
    const parts = [tn('scope.folders', folders.length), formatBytes(totalSize, lang)];
    if (role !== 'owner') {
      parts.push(
        t('scope.shared_by', { owner: scope.owner.display_name, role: t(`role.${role}`) })
      );
    } else if (others.length > 0) {
      const names = others.map((m) => `${m.user.display_name} (${t(`role.${m.role}`)})`);
      parts.push(t('scope.shared_with', { people: joinNames(names, lang) }));
    } else {
      parts.push(t('scope.not_shared'));
    }
    return parts.join(' · ');
  });
</script>

<svelte:head
  ><title>{scope ? `${scope.name} · ${t('app.name')}` : t('app.name')}</title></svelte:head
>

{#if app.scopes && !scope}
  <EmptyState title={t('scope.missing_title')} description={t('scope.missing_body')}>
    {#snippet action()}
      <Button href={resolve('/(app)')}>{t('scope.go_home')}</Button>
    {/snippet}
  </EmptyState>
{:else if loadError}
  <Banner variant="danger">{loadError}</Banner>
{:else}
  <div class="grid items-start gap-x-6 gap-y-[18px] xl:grid-cols-[1fr_390px]">
    <header class="flex flex-wrap items-center gap-3.5 xl:col-span-full">
      <div class="min-w-0 flex-1">
        {#if scope}
          <h1 class="truncate">{scope.name}</h1>
          <p class="text-muted">{subtitle}</p>
        {:else}
          <Skeleton class="h-7 w-48" />
          <Skeleton class="mt-2 h-4 w-72" />
        {/if}
      </div>
      {#if members && members.length > 1}
        <AvatarGroup people={members.map((m) => ({ name: m.user.display_name, key: m.user.id }))} />
      {/if}
      {#if scope}
        <div class="flex gap-2">
          {#if role === 'owner'}
            <Button onclick={() => (sharing = true)}>{t('scope.share')}</Button>
          {/if}
          <Menu items={menu} label={t('common.more_actions')} />
          {#if canEdit}
            <Button variant="primary" onclick={() => (creating = true)}>
              <Plus size={16} aria-hidden="true" />
              {t('folder.new')}
            </Button>
          {/if}
        </div>
      {/if}
    </header>

    {#if conflicted.length > 0}
      <div class="flex flex-col gap-2 xl:col-span-full">
        {#each conflicted as folder (folder.id)}
          <ConflictBanner
            {folder}
            {canEdit}
            {tick}
            oncompare={compare}
            onchanged={() => refreshFolders()}
          />
        {/each}
      </div>
    {/if}

    <div class="grid gap-4 sm:grid-cols-2" role="group" aria-label={t('folder.list_label')}>
      {#if folders === null}
        <Skeleton class="h-[180px] w-full" />
        <Skeleton class="h-[180px] w-full" />
      {:else}
        {#each folders as folder (folder.id)}
          <FolderCard
            {folder}
            selected={folder.id === selectedId}
            onselect={() => select(folder)}
          />
        {/each}
        {#if canEdit}
          <button
            type="button"
            class="card grid min-h-[150px] cursor-pointer place-items-center border-dashed p-[18px] text-muted hover:border-border-strong hover:text-text"
            onclick={() => (creating = true)}
          >
            <span class="flex flex-col items-center gap-1">
              <span class="flex items-center gap-1.5"
                ><FolderPlus size={18} aria-hidden="true" />{t('folder.new')}</span
              >
              <span class="text-xs">{t('folder.new_hint')}</span>
            </span>
          </button>
        {:else if folders.length === 0}
          <div class="sm:col-span-2">
            <EmptyState title={t('folder.none_title')} description={t('folder.none_body')} />
          </div>
        {/if}
      {/if}
    </div>

    {#if selected && scope}
      <FolderPanel
        folder={selected}
        {role}
        bind:tab
        bind:inspect
        {tick}
        onchanged={() => refreshFolders()}
        ontrashed={() => {
          void app.loadScopes();
          refreshFolders();
        }}
      />
    {:else if folders && folders.length > 0}
      <p class="note">{t('folder.select_hint')}</p>
    {/if}
  </div>

  {#if scope}
    {#if role === 'owner'}
      <ShareModal bind:open={sharing} {scope} onchanged={() => refreshMembers()} />
      <PromptModal
        bind:open={renaming}
        title={t('scope.rename')}
        label={t('field.name')}
        value={scope.name}
        submitLabel={t('common.rename')}
        onsubmit={rename}
      />
    {/if}
    <FolderCreateModal bind:open={creating} scopeId={scope.id} oncreated={created} />
  {/if}
{/if}
