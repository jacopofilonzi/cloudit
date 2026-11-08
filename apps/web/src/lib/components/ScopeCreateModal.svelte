<script lang="ts">
  // "New scope": a name, then straight to the new scope's page.
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { Banner, Button, Field, Input, Modal, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { createScope } from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { formError } from '#lib/errors.ts';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let name = $state('');
  let busy = $state(false);
  let error = $state<{ field?: 'name'; form?: string }>({});

  $effect(() => {
    if (open) {
      name = '';
      error = {};
    }
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = {};
    try {
      const scope = await createScope({ name });
      await app.loadScopes().catch(() => undefined);
      toast.success(t('scope.created', { name: scope.name }));
      open = false;
      await goto(resolve('/(app)/scopes/[id]', { id: scope.id }));
    } catch (err) {
      error = formError(err, ['name']);
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={t('scope.create_title')} description={t('scope.create_desc')} size="sm">
  <form id="scope-create" onsubmit={submit} class="flex flex-col gap-3">
    {#if error.form}<Banner variant="danger">{error.form}</Banner>{/if}
    <Field label={t('field.name')} error={error.field ? t('error.name_taken') : undefined}>
      {#snippet children(control)}
        <Input {...control} bind:value={name} required maxlength={100} autocomplete="off" />
      {/snippet}
    </Field>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>{t('common.cancel')}</Button>
    <Button variant="primary" type="submit" form="scope-create" loading={busy}>
      {t('common.create')}
    </Button>
  {/snippet}
</Modal>
