<script lang="ts">
  // "New folder": a name and an optional emoji (editor or more).
  import type { FolderInfo } from '@cloudit/api-types';
  import { Banner, Button, EmojiPicker, Field, Input, Modal, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { createFolder } from '#lib/api.ts';
  import { formError } from '#lib/errors.ts';

  let {
    open = $bindable(false),
    scopeId,
    oncreated
  }: { open?: boolean; scopeId: string; oncreated: (folder: FolderInfo) => void } = $props();

  let name = $state('');
  let emoji = $state('');
  let busy = $state(false);
  let error = $state<{ field?: 'name' | 'emoji'; form?: string }>({});

  $effect(() => {
    if (open) {
      name = '';
      emoji = '';
      error = {};
    }
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = {};
    try {
      const folder = await createFolder(scopeId, { name, emoji: emoji || undefined });
      toast.success(t('folder.created', { name: folder.name }));
      open = false;
      oncreated(folder);
    } catch (err) {
      error = formError(err, ['name', 'emoji']);
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={t('folder.new')} description={t('folder.create_desc')} dismissible={!busy}>
  <form id="folder-create" onsubmit={submit} class="flex flex-col gap-4">
    {#if error.form}<Banner variant="danger">{error.form}</Banner>{/if}
    <Field
      label={t('field.name')}
      error={error.field === 'name' ? t('field.name_invalid') : undefined}
    >
      {#snippet children(control)}
        <Input {...control} bind:value={name} required maxlength={100} autocomplete="off" />
      {/snippet}
    </Field>
    <Field
      label={t('folder.emoji')}
      group
      error={error.field === 'emoji' ? t('emoji.invalid') : undefined}
    >
      <EmojiPicker bind:value={emoji} />
    </Field>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>{t('common.cancel')}</Button>
    <Button variant="primary" type="submit" form="folder-create" loading={busy}>
      {t('common.create')}
    </Button>
  {/snippet}
</Modal>
