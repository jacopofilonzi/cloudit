<script lang="ts">
  // Asks to confirm an action (destructive ones with `danger`). `onconfirm` may be
  // async: the dialog shows a spinner and stays open until it settles.
  import Button from './Button.svelte';
  import Modal from './Modal.svelte';
  import { t } from '../i18n/i18n.svelte';

  let {
    open = $bindable(false),
    title,
    message,
    confirmLabel,
    cancelLabel,
    danger = false,
    onconfirm,
    oncancel
  }: {
    open?: boolean;
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel?: string;
    danger?: boolean;
    onconfirm: () => void | Promise<void>;
    oncancel?: () => void;
  } = $props();

  let busy = $state(false);
  let confirmed = false;

  async function confirm() {
    busy = true;
    try {
      confirmed = true;
      await onconfirm();
      open = false;
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  bind:open
  {title}
  size="sm"
  dismissible={!busy}
  onclose={() => {
    if (!confirmed) oncancel?.();
    confirmed = false;
  }}
>
  <p class="text-muted">{message}</p>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>
      {cancelLabel ?? t('common.cancel')}
    </Button>
    <Button variant={danger ? 'danger-solid' : 'primary'} loading={busy} onclick={confirm}>
      {confirmLabel}
    </Button>
  {/snippet}
</Modal>
