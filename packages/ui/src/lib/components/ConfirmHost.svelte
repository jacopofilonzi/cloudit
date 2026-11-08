<script lang="ts">
  // Renders the requests of `confirmAction()`; mount once in the root layout.
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { answerConfirm, getPendingConfirm } from '../confirm.svelte';

  const pending = $derived(getPendingConfirm());
  // Writable: the dialog closes itself (bind:open) before answering.
  let open = $derived(pending !== null);
</script>

{#if pending}
  <ConfirmDialog
    bind:open
    title={pending.title}
    message={pending.message}
    confirmLabel={pending.confirmLabel}
    cancelLabel={pending.cancelLabel}
    danger={pending.danger}
    onconfirm={() => answerConfirm(true)}
    oncancel={() => answerConfirm(false)}
  />
{/if}
