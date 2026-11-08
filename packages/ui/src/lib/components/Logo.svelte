<script lang="ts">
  // The cloudit mark: a cloud on the accent colour.
  import Cloud from '@lucide/svelte/icons/cloud';

  let { size = 28, class: className = '' }: { size?: number; class?: string } = $props();
</script>

<span
  class={['grid flex-none place-items-center bg-primary text-on-primary', className]}
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{Math.round(size * 0.29)}px"
  aria-hidden="true"
>
  <Cloud size={Math.round(size * 0.55)} fill="currentColor" strokeWidth={1.5} />
</span>
