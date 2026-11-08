<script lang="ts">
  // One linked folder's settings: where it is on this computer, what is ignored,
  // pause, and unlinking (the files stay on the computer and on the server).
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import { page } from '$app/state';
  import { Banner, Button, Input, Toggle, confirmAction, toast } from '@cloudit/ui';
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
          checked={false}
          disabled
          label={t('desktop.fs.auto')}
          description={`${t('desktop.fs.coming_soon')} — ${t('desktop.fs.auto_hint')}`}
        />
      </div>

      <div class="mt-4 flex items-center gap-3 border-t border-border pt-3.5">
        <Button variant="danger" onclick={unlink}>{t('desktop.fs.unlink')}</Button>
        <span class="text-xs text-muted">{t('desktop.fs.unlink_note')}</span>
      </div>
    </div>
  {/if}
</div>
