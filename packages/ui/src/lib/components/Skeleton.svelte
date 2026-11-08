<script lang="ts">
  // Placeholder block while content loads; size it with classes (`h-4 w-32`).
  let { class: className = '' }: { class?: string } = $props();
</script>

<span class={['block animate-pulse rounded-md bg-surface-alt', className]} aria-hidden="true"
></span>
