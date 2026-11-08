<script lang="ts">
  // A modal with one text field (rename a scope or device, label a version, pick a new
  // name when a restore hits `name_taken`). `onsubmit` does the work; when it throws,
  // the error shows in the dialog and it stays open.
  import { Banner, Button, Field, Input, Modal } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { errorText } from '#lib/errors.ts';

  let {
    open = $bindable(false),
    title,
    description,
    label,
    hint,
    value = '',
    submitLabel,
    allowEmpty = false,
    maxlength = 100,
    onsubmit
  }: {
    open?: boolean;
    title: string;
    description?: string;
    label: string;
    hint?: string;
    /** The initial text, read each time the dialog opens. */
    value?: string;
    submitLabel: string;
    allowEmpty?: boolean;
    maxlength?: number;
    onsubmit: (value: string) => Promise<void>;
  } = $props();

  let text = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    if (open) {
      text = value;
      error = null;
    }
  });

  const uid = $props.id();

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = null;
    try {
      await onsubmit(text.trim());
      open = false;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open {title} {description} size="sm" dismissible={!busy}>
  <form id="prompt-{uid}" onsubmit={submit} class="flex flex-col gap-3">
    {#if error}<Banner variant="danger">{error}</Banner>{/if}
    <Field {label} {hint}>
      {#snippet children(control)}
        <Input
          {...control}
          bind:value={text}
          required={!allowEmpty}
          {maxlength}
          autocomplete="off"
          autofocus
        />
      {/snippet}
    </Field>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>{t('common.cancel')}</Button>
    <Button variant="primary" type="submit" form="prompt-{uid}" loading={busy}>
      {submitLabel}
    </Button>
  {/snippet}
</Modal>
