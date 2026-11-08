<script lang="ts">
  // One sidebar entry: a link (current page highlighted) or plain text.
  import type { Snippet } from 'svelte';

  let {
    href,
    current = false,
    count,
    badge,
    children
  }: {
    href?: string;
    current?: boolean;
    count?: number;
    /** Red counter (things waiting for the user). */
    badge?: number;
    children: Snippet;
  } = $props();

  const classes = $derived([
    'flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-text no-underline',
    href && 'hover:bg-surface-alt',
    current && 'bg-primary-soft font-semibold text-primary-fg hover:bg-primary-soft'
  ]);
</script>

{#snippet content()}
  {@render children()}
  {#if badge}
    <span class="ms-auto rounded-full bg-danger px-[7px] text-[11px] text-on-danger">{badge}</span>
  {:else if count !== undefined}
    <span class="ms-auto text-xs text-muted">{count}</span>
  {/if}
{/snippet}

{#if href}
  <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- callers pass resolve()d hrefs -->
  <a {href} class={classes} aria-current={current ? 'page' : undefined}>{@render content()}</a>
{:else}
  <div class={classes}>{@render content()}</div>
{/if}
