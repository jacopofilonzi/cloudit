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
  let trigger = $state<HTMLButtonElement>();
  let menu = $state<HTMLElement>();
  // The list is `position: fixed` (placed from the trigger's rectangle) so a scrolling
  // ancestor such as a table wrapper with `overflow-x: auto` can't clip it.
  let position = $state({ top: 0, right: 0 });

  const GAP = 6;
  const MARGIN = 8;

  function place() {
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const height = menu?.offsetHeight ?? 0;
    const below = rect.bottom + GAP;
    // Open upwards when the list wouldn't fit below the trigger but fits above it.
    const top =
      below + height > window.innerHeight - MARGIN && rect.top - GAP - height >= MARGIN
        ? rect.top - GAP - height
        : below;
    position = { top, right: Math.max(MARGIN, window.innerWidth - rect.right) };
  }

  $effect(() => {
    if (open && menu) place();
  });
</script>

<svelte:window
  onclick={(event) => {
    if (open && root && event.target instanceof Node && !root.contains(event.target)) open = false;
  }}
  onkeydown={(event) => {
    if (event.key === 'Escape') open = false;
  }}
  onresize={() => {
    if (open) place();
  }}
  onscrollcapture={() => {
    if (open) place();
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
    bind:this={trigger}
    onclick={() => (open = !open)}
  >
    <Ellipsis size={16} aria-hidden="true" />
  </button>
  {#if open}
    <div
      role="menu"
      bind:this={menu}
      style:top="{position.top}px"
      style:right="{position.right}px"
      class="card fixed z-50 min-w-48 p-1 whitespace-nowrap shadow-[0_12px_30px_var(--cl-shadow)]"
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
