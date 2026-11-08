<script lang="ts">
  // The folder's emoji on a tile, or a plain folder icon when it has none.
  import Folder from '@lucide/svelte/icons/folder';

  let { emoji, size = 34 }: { emoji: string | null; size?: number } = $props();
</script>

<span
  class="grid flex-none place-items-center rounded-[9px] bg-surface-alt"
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.5)}px"
  aria-hidden="true"
>
  {#if emoji}{emoji}{:else}<Folder size={Math.round(size * 0.5)} class="text-muted" />{/if}
</span>
