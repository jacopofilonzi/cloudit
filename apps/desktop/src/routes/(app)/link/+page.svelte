<script lang="ts">
  // Link a local folder to a server folder in three steps: the folder on this computer,
  // the folder on the server (new or existing, in a new or existing collection), and
  // the first sync.
  import { onMount } from 'svelte';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import type { FolderInfo, LinkedFolder, ScopeInfo, SyncReport } from '@cloudit/api-types';
  import {
    Banner,
    Button,
    EmojiPicker,
    Field,
    Input,
    RadioCards,
    Select,
    Spinner
  } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { showConflict, showFolders } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import IgnoreEditor from '#lib/components/IgnoreEditor.svelte';
  import OpProgress from '#lib/components/OpProgress.svelte';
  import { errorText } from '#lib/errors.ts';

  type Step = 1 | 2 | 3;
  const NEW_SCOPE = '__new__';

  let step = $state<Step>(1);
  let localPath = $state('');
  let scopes = $state<ScopeInfo[] | null>(null);
  let scopeChoice = $state('');
  let newScopeName = $state('');
  let mode = $state<'new' | 'existing'>('new');
  let folderName = $state('');
  let emoji = $state('');
  let serverFolders = $state<FolderInfo[]>([]);
  let serverFolder = $state('');
  let ignore = $state<string[]>([]);
  let error = $state<string | null>(null);

  // Step 3
  let working = $state(false);
  let linked = $state<LinkedFolder | null>(null);
  let report = $state<SyncReport | null>(null);

  const stepLabels = [
    'desktop.link.step_local',
    'desktop.link.step_server',
    'desktop.link.step_sync'
  ] as const;

  onMount(async () => {
    try {
      scopes = await api.scopes();
      scopeChoice = scopes[0]?.id ?? NEW_SCOPE;
    } catch (err) {
      app.noteError(err);
      error = errorText(err);
      scopes = [];
      scopeChoice = NEW_SCOPE;
    }
  });

  // The folders of the chosen collection, for "existing folder".
  $effect(() => {
    const scope = scopeChoice;
    if (!scope || scope === NEW_SCOPE) {
      serverFolders = [];
      return;
    }
    let current = true;
    api.serverFolders(scope).then(
      (list) => {
        if (!current) return;
        serverFolders = list;
        serverFolder = list.find((f) => !isLinked(f.id))?.id ?? '';
      },
      (err: unknown) => {
        if (current) error = errorText(err);
      }
    );
    return () => (current = false);
  });

  const isLinked = (folderId: string) => app.folders.some((f) => f.folder_id === folderId);
  const scopeReady = $derived(scopeChoice !== NEW_SCOPE || newScopeName.trim() !== '');
  const folderReady = $derived(
    mode === 'new' ? folderName.trim() !== '' : serverFolder !== '' && scopeChoice !== NEW_SCOPE
  );
  const scopeOptions = $derived([
    ...(scopes ?? []).map((s) => ({ value: s.id, label: s.name })),
    { value: NEW_SCOPE, label: t('desktop.link.collection_new') }
  ]);
  const folderOptions = $derived(
    serverFolders.map((f) => ({
      value: f.id,
      label: isLinked(f.id) ? `${f.name} (✓)` : f.name
    }))
  );

  async function browse() {
    const picked = await api.pickFolder(t('desktop.link.pick_title'), localPath || undefined);
    if (picked) {
      localPath = picked;
      if (!folderName.trim()) folderName = picked.split(/[\\/]/).filter(Boolean).pop() ?? '';
    }
  }

  async function start() {
    step = 3;
    error = null;
    working = true;
    linked = null;
    report = null;
    try {
      let scopeId = scopeChoice;
      if (scopeId === NEW_SCOPE) {
        const created = await api.createScope(newScopeName.trim());
        scopeId = created.id;
        scopes = [...(scopes ?? []), created];
        scopeChoice = created.id;
      }
      linked =
        mode === 'new'
          ? await api.createAndLink(scopeId, folderName.trim(), emoji || null, localPath, ignore)
          : await api.linkFolder(serverFolder, localPath, ignore);
      await app.loadFolders();
      app.selected = linked.id;
      report = await api.syncFolder(linked.id);
    } catch (err) {
      app.noteError(err);
      error = errorText(err);
    } finally {
      working = false;
      await app.loadFolders();
    }
  }

  const needsDecision = $derived(report?.outcome.outcome === 'needs_decision');
</script>

<div class="mx-auto max-w-[560px]">
  <div class="card p-[18px]">
    <ol class="mb-3.5 flex gap-2 text-xs">
      {#each stepLabels as label, i (label)}
        <li
          class={[
            'rounded-full px-2.5 py-[3px]',
            step === i + 1
              ? 'bg-primary text-on-primary'
              : step > i + 1
                ? 'bg-success-soft text-success'
                : 'bg-surface-alt text-muted'
          ]}
          aria-current={step === i + 1 ? 'step' : undefined}
        >
          {i + 1} · {t(label)}
        </li>
      {/each}
    </ol>

    {#if error && step !== 3}
      <Banner variant="danger" class="mb-3">{error}</Banner>
    {/if}

    {#if step === 1}
      <h2>{t('desktop.link.title')}</h2>
      <div class="mt-3">
        <Field label={t('desktop.link.local')} hint={t('desktop.link.local_hint')}>
          {#snippet children(control)}
            <div class="flex gap-1.5">
              <Input {...control} bind:value={localPath} class="font-mono text-[12px]" />
              <Button onclick={browse}>{t('desktop.link.browse')}</Button>
            </div>
          {/snippet}
        </Field>
      </div>
      <div class="mt-4 flex justify-between">
        <Button variant="ghost" onclick={showFolders}>{t('common.cancel')}</Button>
        <Button
          variant="primary"
          disabled={localPath.trim() === ''}
          onclick={() => {
            error = null;
            step = 2;
          }}
        >
          {t('desktop.link.next')}
        </Button>
      </div>
    {:else if step === 2}
      <h2>{t('desktop.link.step_server')}</h2>
      {#if scopes === null}
        <div class="mt-4 flex items-center gap-2 text-muted">
          <Spinner size={14} />{t('desktop.link.loading')}
        </div>
      {:else}
        <div class="mt-3 space-y-3">
          <Field label={t('desktop.link.collection')}>
            {#snippet children(control)}
              <Select {...control} bind:value={scopeChoice} options={scopeOptions} />
            {/snippet}
          </Field>
          {#if scopeChoice === NEW_SCOPE}
            <Field label={t('desktop.link.collection_name')}>
              {#snippet children(control)}
                <Input {...control} bind:value={newScopeName} maxlength={100} />
              {/snippet}
            </Field>
          {/if}

          <RadioCards
            name="link-mode"
            bind:value={mode}
            options={[
              {
                value: 'new',
                title: t('desktop.link.mode_new'),
                description: t('desktop.link.mode_new_hint')
              },
              {
                value: 'existing',
                title: t('desktop.link.mode_existing'),
                description: t('desktop.link.mode_existing_hint')
              }
            ]}
          />

          {#if mode === 'new'}
            <Field label={t('desktop.link.folder_name')}>
              {#snippet children(control)}
                <Input {...control} bind:value={folderName} maxlength={100} />
              {/snippet}
            </Field>
            <Field label={t('desktop.link.folder_icon')} group>
              <EmojiPicker bind:value={emoji} />
            </Field>
          {:else if scopeChoice !== NEW_SCOPE}
            {#if folderOptions.length === 0}
              <p class="text-muted">{t('desktop.link.no_folders')}</p>
            {:else}
              <Field label={t('desktop.link.folder_choose')}>
                {#snippet children(control)}
                  <Select {...control} bind:value={serverFolder} options={folderOptions} />
                {/snippet}
              </Field>
            {/if}
          {/if}

          <Field label={t('desktop.link.ignore')} group>
            <IgnoreEditor patterns={ignore} onchange={(next) => (ignore = next)} />
          </Field>
        </div>
        <div class="mt-4 flex justify-between">
          <Button onclick={() => (step = 1)}>{t('desktop.link.back')}</Button>
          <Button variant="primary" disabled={!scopeReady || !folderReady} onclick={start}>
            {t('desktop.link.start')}
          </Button>
        </div>
      {/if}
    {:else}
      <h2>{t('desktop.link.step_sync')}</h2>
      {#if working}
        <div class="mt-3">
          <p class="mb-2 text-muted">{t('desktop.link.syncing')}</p>
          <OpProgress progress={linked ? app.progress[linked.id] : undefined} />
        </div>
      {:else if error}
        <Banner
          variant={linked ? 'warning' : 'danger'}
          class="mt-3"
          title={linked ? t('desktop.link.problem_title') : undefined}
        >
          {error}
        </Banner>
        <div class="mt-4 flex justify-between">
          {#if linked}
            <span></span>
            <Button variant="primary" onclick={showFolders}>{t('desktop.link.to_folders')}</Button>
          {:else}
            <Button onclick={() => (step = 2)}>{t('desktop.link.back')}</Button>
            <Button variant="primary" onclick={start}>{t('desktop.action.retry')}</Button>
          {/if}
        </div>
      {:else if linked}
        <div class="mt-3 flex items-start gap-3">
          <CircleCheck
            size={22}
            class={needsDecision ? 'text-warning' : 'text-success'}
            aria-hidden="true"
          />
          <div>
            <b class="block">
              {needsDecision ? t('desktop.link.conflict_title') : t('desktop.link.done_title')}
            </b>
            <p class="text-muted">
              {needsDecision
                ? t('desktop.link.conflict_body')
                : t('desktop.link.done_body', { name: linked.folder_name })}
            </p>
          </div>
        </div>
        <div class="mt-4 flex justify-end gap-2">
          {#if needsDecision}
            <Button onclick={showFolders}>{t('desktop.link.to_folders')}</Button>
            <Button variant="primary" onclick={() => linked && showConflict(linked.id)}>
              {t('desktop.link.resolve')}
            </Button>
          {:else}
            <Button variant="primary" onclick={showFolders}>{t('desktop.link.to_folders')}</Button>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</div>
