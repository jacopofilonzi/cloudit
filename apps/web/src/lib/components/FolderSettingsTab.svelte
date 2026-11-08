<script lang="ts">
  // Folder settings: name and emoji (editor or more), retention overrides and the
  // trash (admin or more). Lower roles see the values without the controls.
  import type { FolderInfo, ScopeRole, UpdateFolderRequest } from '@cloudit/api-types';
  import { Banner, Button, EmojiPicker, Field, Input, confirmAction, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { trashFolder, updateFolder } from '#lib/api.ts';
  import { errorText, formError } from '#lib/errors.ts';
  import { atLeast } from '#lib/roles.ts';

  let {
    folder,
    role,
    onchanged,
    ontrashed
  }: {
    folder: FolderInfo;
    role: ScopeRole;
    onchanged: () => void;
    ontrashed: () => void;
  } = $props();

  const canEdit = $derived(atLeast(role, 'editor'));
  const canRetain = $derived(atLeast(role, 'admin'));

  let name = $state('');
  let emoji = $state('');
  let maxVersions = $state('');
  let maxPerDay = $state('');
  let busy = $state(false);
  let error = $state<{ field?: string; form?: string }>({});

  // Fields restart from the folder whenever another folder (or a newer copy) is shown.
  $effect(() => {
    name = folder.name;
    emoji = folder.emoji ?? '';
    maxVersions = folder.retention_max_versions?.toString() ?? '';
    maxPerDay = folder.retention_max_per_day?.toString() ?? '';
    error = {};
  });

  /** `''` = server default (`null`), else a whole number 0..=100000; `undefined` = invalid. */
  function parseLimit(text: string): number | null | undefined {
    const trimmed = text.trim();
    if (trimmed === '') return null;
    if (!/^\d{1,6}$/u.test(trimmed)) return undefined;
    const value = Number(trimmed);
    return value <= 100_000 ? value : undefined;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    error = {};
    const body: UpdateFolderRequest = {};
    if (canEdit) {
      if (name.trim() !== folder.name) body.name = name;
      if (emoji !== (folder.emoji ?? '')) body.emoji = emoji === '' ? null : emoji;
    }
    if (canRetain) {
      const versions = parseLimit(maxVersions);
      const perDay = parseLimit(maxPerDay);
      if (versions === undefined) return void (error = { field: 'retention_max_versions' });
      if (perDay === undefined) return void (error = { field: 'retention_max_per_day' });
      if (versions !== folder.retention_max_versions) body.retention_max_versions = versions;
      if (perDay !== folder.retention_max_per_day) body.retention_max_per_day = perDay;
    }
    if (Object.keys(body).length === 0) return;
    busy = true;
    try {
      await updateFolder(folder.id, body);
      toast.success(t('common.saved'));
      onchanged();
    } catch (err) {
      error = formError(err, ['name', 'emoji', 'retention_max_versions', 'retention_max_per_day']);
    } finally {
      busy = false;
    }
  }

  async function trash() {
    const ok = await confirmAction({
      title: t('settings.trash_title', { name: folder.name }),
      message: t('settings.trash_body'),
      confirmLabel: t('settings.trash'),
      danger: true
    });
    if (!ok) return;
    try {
      await trashFolder(folder.id);
      toast.success(t('settings.trashed'));
      ontrashed();
    } catch (err) {
      toast.error(errorText(err));
    }
  }
</script>

{#if !canEdit}
  <p class="note mb-3">{t('settings.read_only')}</p>
{/if}

<form onsubmit={save} class="flex flex-col gap-4">
  {#if error.form}<Banner variant="danger">{error.form}</Banner>{/if}

  <Field
    label={t('field.name')}
    error={error.field === 'name' ? t('field.name_invalid') : undefined}
  >
    {#snippet children(control)}
      <Input
        {...control}
        bind:value={name}
        required
        maxlength={100}
        readonly={!canEdit}
        autocomplete="off"
      />
    {/snippet}
  </Field>

  {#if canEdit}
    <Field
      label={t('folder.emoji')}
      group
      error={error.field === 'emoji' ? t('emoji.invalid') : undefined}
    >
      <EmojiPicker bind:value={emoji} />
    </Field>
  {/if}

  <fieldset class="flex flex-col gap-3 border-t border-border pt-4">
    <legend class="sr-only">{t('settings.retention')}</legend>
    <h3 class="text-sm font-semibold">{t('settings.retention')}</h3>
    <div class="grid grid-cols-2 gap-3">
      <Field
        label={t('settings.max_versions')}
        error={error.field === 'retention_max_versions'
          ? t('settings.retention_invalid')
          : undefined}
      >
        {#snippet children(control)}
          <Input
            {...control}
            bind:value={maxVersions}
            inputmode="numeric"
            readonly={!canRetain}
            placeholder={String(folder.effective_retention_max_versions)}
          />
        {/snippet}
      </Field>
      <Field
        label={t('settings.max_per_day')}
        error={error.field === 'retention_max_per_day'
          ? t('settings.retention_invalid')
          : undefined}
      >
        {#snippet children(control)}
          <Input
            {...control}
            bind:value={maxPerDay}
            inputmode="numeric"
            readonly={!canRetain}
            placeholder={String(folder.effective_retention_max_per_day)}
          />
        {/snippet}
      </Field>
    </div>
    <p class="hint">
      {canRetain ? t('settings.default_hint') : t('settings.retention_admin')}
      {t('settings.retention_note')}
    </p>
  </fieldset>

  {#if canEdit || canRetain}
    <div><Button variant="primary" type="submit" loading={busy}>{t('common.save')}</Button></div>
  {/if}
</form>

{#if canRetain}
  <div class="mt-6 border-t border-border pt-4">
    <h3 class="mb-1 text-sm font-semibold">{t('settings.trash')}</h3>
    <p class="hint mb-3">{t('settings.trash_body')}</p>
    <Button variant="danger" onclick={trash}>{t('settings.trash')}</Button>
  </div>
{/if}
