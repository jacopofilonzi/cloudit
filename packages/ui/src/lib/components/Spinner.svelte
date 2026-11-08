<script lang="ts">
  // A small rotating indicator. Decorative unless `label` is given.
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';

  let {
    size = 16,
    label,
    class: className = ''
  }: { size?: number; label?: string; class?: string } = $props();
</script>

{#if label}
  <span role="status" class={['inline-flex items-center gap-2 text-muted', className]}>
    <LoaderCircle {size} class="animate-spin" aria-hidden="true" />
    <span>{label}</span>
  </span>
{:else}
  <LoaderCircle {size} class={['animate-spin', className]} aria-hidden="true" />
{/if}
