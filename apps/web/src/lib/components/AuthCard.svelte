<script lang="ts">
  // The centred card of the pages outside the app (login, setup, invite…).
  import type { Snippet } from 'svelte';
  import { Logo } from '@cloudit/ui';

  let {
    title,
    subtitle,
    wide = false,
    center = false,
    header,
    children
  }: {
    title: string;
    subtitle?: string | Snippet;
    wide?: boolean;
    center?: boolean;
    /** Replaces the logo above the title. */
    header?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<div class={['card w-full p-8', wide ? 'max-w-[520px]' : 'max-w-[400px]', center && 'text-center']}>
  {#if header}{@render header()}{:else}<Logo size={40} class={center ? 'mx-auto' : ''} />{/if}
  <h1 class="mt-4 mb-1 text-[22px]">{title}</h1>
  {#if subtitle}
    <div class="text-muted">
      {#if typeof subtitle === 'string'}{subtitle}{:else}{@render subtitle()}{/if}
    </div>
  {/if}
  {@render children?.()}
</div>
