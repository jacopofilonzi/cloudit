<script lang="ts">
  // One linked folder's settings: where it is on this computer, what is ignored,
  // pause, automatic sync (quiet period, associated program), and unlinking (the files
  // stay on the computer and on the server).
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import { page } from '$app/state';
  import type { AutoSyncSettings } from '@cloudit/api-types';
  import { Banner, Button, Input, Segmented, Toggle, confirmAction, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { reportError, showFolders } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import FolderIcon from '#lib/components/FolderIcon.svelte';
  import IgnoreEditor from '#lib/components/IgnoreEditor.svelte';
  import { displayState } from '#lib/status.ts';

  const id = $derived(Number(page.params.id));
  const link = $derived(app.folders.find((f) => f.id === id));
  let saving = $state(false);

  async function pickPath() {
    if (!link) return;
    const picked = await api.pickFolder(t('desktop.fs.pick_title'), link.local_path);
    if (!picked) return;
    saving = true;
    try {
      await api.relocate(id, picked);
      await app.loadFolders();
      toast.success(t('desktop.fs.path_saved'));
      void api.refreshStatus(id).catch((error: unknown) => app.noteError(error));
    } catch (error) {
      reportError(error);
    } finally {
      saving = false;
    }
  }

  async function saveIgnore(patterns: string[]) {
    saving = true;
    try {
      await api.setIgnore(id, patterns);
      await app.loadFolders();
      toast.success(t('desktop.fs.ignore_saved'));
      void api.refreshStatus(id).catch((error: unknown) => app.noteError(error));
    } catch (error) {
      reportError(error);
    } finally {
      saving = false;
    }
  }

  async function togglePause(paused: boolean) {
    try {
      await api.setPaused(id, paused);
      await app.loadFolders();
    } catch (error) {
      reportError(error);
    }
  }

  /** The quiet periods offered (plus the current one if it is another value). */
  const QUIET_CHOICES = [10, 30, 60, 120, 300];
  const quietLabel = (secs: number) =>
    secs < 60 || secs % 60 !== 0
      ? t('desktop.fs.seconds', { count: secs })
      : t('desktop.fs.minutes', { count: secs / 60 });
  const quietOptions = $derived(
    [...new Set([...QUIET_CHOICES, link?.quiet_secs ?? 30])]
      .sort((a, b) => a - b)
      .map((secs) => ({ value: String(secs), label: quietLabel(secs) }))
  );

  // Edited locally; back to the saved name whenever it changes.
  let processDraft = $derived(link?.process_name ?? '');
  let picking = $state(false);
  let processes = $state<string[] | null>(null);
  let filter = $state('');
  const shown = $derived(
    (processes ?? []).filter((name) => name.toLowerCase().includes(filter.trim().toLowerCase()))
  );

  async function saveAuto(patch: Partial<AutoSyncSettings>, done?: string) {
    if (!link) return;
    saving = true;
    try {
      await api.setAutoSync(id, {
        enabled: link.auto_sync,
        quiet_secs: link.quiet_secs,
        process_name: link.process_name,
        ...patch
      });
      await app.loadFolders();
      if (done) toast.success(done);
    } catch (error) {
      reportError(error);
    } finally {
      saving = false;
    }
  }

  function saveProcess(name: string) {
    const trimmed = name.trim();
    return saveAuto({ process_name: trimmed === '' ? null : trimmed }, t('desktop.fs.auto_saved'));
  }

  async function togglePicker() {
    picking = !picking;
    if (!picking) return;
    filter = '';
    processes = null;
    try {
      processes = await api.runningProcesses();
    } catch (error) {
      picking = false;
      reportError(error);
    }
  }

  async function pickProcess(name: string) {
    picking = false;
    processDraft = name;
    await saveProcess(name);
  }

  async function unlink() {
    if (!link) return;
    const ok = await confirmAction({
      title: t('desktop.fs.unlink_title', { name: link.folder_name }),
      message: t('desktop.fs.unlink_body'),
      confirmLabel: t('desktop.fs.unlink_confirm'),
      danger: true
    });
    if (!ok) return;
    try {
      await api.unlink(id);
      await app.loadFolders();
      delete app.statuses[id];
      await showFolders();
    } catch (error) {
      reportError(error);
    }
  }
</script>

<div class="mx-auto max-w-[620px]">
  {#if !link}
    <Banner variant="info">{t('desktop.error.not_linked')}</Banner>
    <div class="mt-3"><Button onclick={showFolders}>{t('desktop.conflict.back')}</Button></div>
  {:else}
    <div class="card p-[18px]">
      <div class="flex items-center gap-2.5">
        <FolderIcon emoji={link.emoji} />
        <div class="min-w-0">
          <b class="block truncate text-[16px]">{link.folder_name}</b>
          <span class="block truncate text-xs text-muted">{link.scope_name}</span>
        </div>
        <Button size="sm" class="ms-auto" onclick={showFolders}>
          <ArrowLeft size={13} aria-hidden="true" />{t('desktop.conflict.back')}
        </Button>
      </div>

      {#if displayState(link, app.statuses[id]) === 'missing_local_path'}
        <Banner variant="warning" class="mt-3">
          {t('desktop.state_hint.missing_local_path')}
        </Banner>
      {/if}

      <div class="mt-4">
        <span class="field-label">{t('desktop.fs.path')}</span>
        <div class="flex gap-1.5">
          <Input value={link.local_path} readonly class="font-mono text-[12px]" />
          <Button disabled={saving} onclick={pickPath}>{t('desktop.fs.change')}</Button>
        </div>
      </div>

      <div class="mt-4">
        <span class="field-label">{t('desktop.fs.ignore')}</span>
        <IgnoreEditor patterns={link.ignore} onchange={saveIgnore} disabled={saving} />
      </div>

      <div class="mt-3 border-t border-border pt-3">
        <Toggle
          checked={link.paused}
          label={t('desktop.fs.pause')}
          description={t('desktop.fs.pause_hint')}
          onchange={togglePause}
        />
      </div>
      <div class="mt-3 border-t border-border pt-3">
        <Toggle
          checked={link.auto_sync}
          disabled={saving}
          label={t('desktop.fs.auto')}
          description={t('desktop.fs.auto_hint')}
          onchange={(enabled) => saveAuto({ enabled })}
        />
        {#if link.auto_sync && !app.autoSync}
          <Banner variant="warning">{t('desktop.fs.auto_global_off')}</Banner>
        {/if}
        {#if link.auto_sync}
          <div class="mt-2">
            <span class="field-label">{t('desktop.fs.quiet')}</span>
            <Segmented
              name="quiet"
              variant="chips"
              bind:value={
                () => String(link?.quiet_secs ?? 30),
                (v) => void saveAuto({ quiet_secs: Number(v) })
              }
              options={quietOptions}
            />
            <p class="hint">{t('desktop.fs.quiet_hint')}</p>
          </div>

          <div class="mt-4">
            <label class="field-label" for="process-name">{t('desktop.fs.process')}</label>
            <div class="flex gap-1.5">
              <Input
                id="process-name"
                bind:value={processDraft}
                placeholder={t('desktop.fs.process_placeholder')}
                class="font-mono text-[12px]"
                onkeydown={(e) => {
                  if (e.key === 'Enter') void saveProcess(processDraft);
                }}
              />
              <Button
                disabled={saving || processDraft.trim() === (link.process_name ?? '')}
                onclick={() => saveProcess(processDraft)}
              >
                {t('desktop.fs.process_save')}
              </Button>
              {#if link.process_name}
                <Button variant="ghost" disabled={saving} onclick={() => saveProcess('')}>
                  {t('desktop.fs.process_clear')}
                </Button>
              {/if}
            </div>
            <p class="hint">{t('desktop.fs.process_hint')}</p>
            <button
              type="button"
              class="mt-1 text-xs text-primary-fg hover:underline"
              aria-expanded={picking}
              onclick={togglePicker}
            >
              {picking ? t('desktop.fs.process_close') : t('desktop.fs.process_pick')}
            </button>
            {#if picking}
              <div class="mt-2 rounded-lg border border-border p-2">
                <Input
                  bind:value={filter}
                  placeholder={t('desktop.fs.process_filter')}
                  aria-label={t('desktop.fs.process_filter')}
                />
                {#if processes === null}
                  <p class="mt-2 text-xs text-muted">{t('desktop.fs.process_loading')}</p>
                {:else if shown.length === 0}
                  <p class="mt-2 text-xs text-muted">{t('desktop.fs.process_empty')}</p>
                {:else}
                  <ul class="mt-1.5 max-h-44 overflow-auto">
                    {#each shown as name (name)}
                      <li>
                        <button
                          type="button"
                          class="w-full rounded-md px-2 py-1 text-left font-mono text-[12px] hover:bg-surface-alt"
                          onclick={() => pickProcess(name)}
                        >
                          {name}
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <div class="mt-4 flex items-center gap-3 border-t border-border pt-3.5">
        <Button variant="danger" onclick={unlink}>{t('desktop.fs.unlink')}</Button>
        <span class="text-xs text-muted">{t('desktop.fs.unlink_note')}</span>
      </div>
    </div>
  {/if}
</div>
