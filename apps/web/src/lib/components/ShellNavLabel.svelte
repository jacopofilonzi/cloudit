<script lang="ts">
  // A sidebar section title (uppercase), with an optional action on the right.
  import type { Snippet } from 'svelte';

  let { children, action }: { children: Snippet; action?: Snippet } = $props();
</script>

<div class="flex items-center px-2 pt-3.5 pb-1.5 text-[11px] tracking-[.08em] text-muted uppercase">
  {@render children()}
  {#if action}<span class="ms-auto">{@render action()}</span>{/if}
</div>
