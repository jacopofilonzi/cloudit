<script lang="ts">
  // A "more actions" button that opens a short list of actions. Closes on a click
  // outside, on Escape and after choosing an item.
  import type { Component } from 'svelte';
  import Ellipsis from '@lucide/svelte/icons/ellipsis';

  interface Item {
    label: string;
    onclick: () => void;
    danger?: boolean;
    /** A lucide icon component. */
    icon?: Component<{ size?: number; class?: string }>;
  }

  let {
    items,
    label,
    class: className = ''
  }: { items: readonly Item[]; label: string; class?: string } = $props();

  let open = $state(false);
  let root = $state<HTMLElement>();
</script>

<svelte:window
  onclick={(event) => {
    if (open && root && event.target instanceof Node && !root.contains(event.target)) open = false;
  }}
  onkeydown={(event) => {
    if (event.key === 'Escape') open = false;
  }}
/>

<div class={['relative', className]} bind:this={root}>
  <button
    type="button"
    class="btn btn-secondary"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={label}
    title={label}
    onclick={() => (open = !open)}
  >
    <Ellipsis size={16} aria-hidden="true" />
  </button>
  {#if open}
    <div
      role="menu"
      class="card absolute right-0 z-30 mt-1.5 min-w-48 p-1 shadow-[0_12px_30px_var(--cl-shadow)]"
    >
      {#each items as item (item.label)}
        <button
          type="button"
          role="menuitem"
          class={[
            'flex w-full cursor-pointer items-center gap-2 rounded-md px-2.5 py-2 text-left hover:bg-surface-alt',
            item.danger ? 'text-danger-fg' : 'text-text'
          ]}
          onclick={() => {
            open = false;
            item.onclick();
          }}
        >
          {#if item.icon}<item.icon size={15} class="flex-none" />{/if}
          {item.label}
        </button>
      {/each}
    </div>
  {/if}
</div>
