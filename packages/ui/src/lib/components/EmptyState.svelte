<script lang="ts">
  import type { Component, Snippet } from 'svelte';

  let {
    icon: Icon,
    title,
    description,
    action,
    class: className = ''
  }: {
    /** A lucide icon component. */
    icon?: Component<{ size?: number; class?: string }>;
    title: string;
    description?: string;
    action?: Snippet;
    class?: string;
  } = $props();
</script>

<div
  class={[
    'flex flex-col items-center gap-2 rounded-xl border border-dashed border-border-strong px-6 py-10 text-center',
    className
  ]}
>
  {#if Icon}
    <span class="icon-tile mb-1 size-11 text-muted"><Icon size={22} /></span>
  {/if}
  <p class="font-semibold">{title}</p>
  {#if description}<p class="max-w-sm text-muted">{description}</p>{/if}
  {#if action}<div class="mt-2">{@render action()}</div>{/if}
</div>
