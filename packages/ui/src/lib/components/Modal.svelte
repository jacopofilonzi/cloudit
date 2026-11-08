<script lang="ts">
  // A modal dialog on the native <dialog> element: `showModal()` makes the rest of
  // the page inert (focus stays inside), Esc closes it, focus returns to the
  // element that opened it. A click on the backdrop closes it too.
  import type { Snippet } from 'svelte';
  import X from '@lucide/svelte/icons/x';
  import { t } from '../i18n/i18n.svelte';

  let {
    open = $bindable(false),
    title,
    description,
    size = 'md',
    dismissible = true,
    onclose,
    children,
    footer
  }: {
    open?: boolean;
    title: string;
    description?: string;
    size?: 'sm' | 'md' | 'lg';
    /** Esc, backdrop and the close button close it (off while an action runs). */
    dismissible?: boolean;
    onclose?: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  const uid = $props.id();
  let dialog = $state<HTMLDialogElement>();

  const widths = { sm: 'max-w-[420px]', md: 'max-w-[520px]', lg: 'max-w-[680px]' };

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });

  function close() {
    if (dismissible) open = false;
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="{uid}-title"
  aria-describedby={description ? `${uid}-desc` : undefined}
  class={[
    'card m-auto w-[calc(100%-2rem)] p-0 text-text shadow-[0_30px_80px_var(--cl-shadow)] backdrop:bg-overlay',
    widths[size]
  ]}
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
  onclose={() => {
    open = false;
    onclose?.();
  }}
  onclick={(event) => {
    if (event.target === dialog) close();
  }}
>
  {#if open}
    <div class="p-6">
      <div class="mb-1 flex items-start gap-3">
        <h2 id="{uid}-title" class="min-w-0 flex-1">{title}</h2>
        {#if dismissible}
          <button
            type="button"
            class="btn btn-ghost btn-sm -mt-1 -mr-2"
            aria-label={t('common.close')}
            onclick={close}
          >
            <X size={16} aria-hidden="true" />
          </button>
        {/if}
      </div>
      {#if description}<p id="{uid}-desc" class="text-xs text-muted">{description}</p>{/if}
      <div class="mt-3">{@render children()}</div>
      {#if footer}
        <div class="mt-5 flex flex-wrap justify-end gap-2">{@render footer()}</div>
      {/if}
    </div>
  {/if}
</dialog>
