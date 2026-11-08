<script lang="ts">
  // A data table in a card: pass `<thead>` / `<tbody>` rows as children. Cells get
  // the mockup's padding and separators from the `data-table` utility; use
  // `TableActions` in the last cell for right-aligned buttons.
  import type { Snippet } from 'svelte';

  let {
    caption,
    children,
    class: className = ''
  }: {
    /** Accessible name of the table (visually hidden). */
    caption: string;
    children: Snippet;
    class?: string;
  } = $props();
</script>

<div class={['card overflow-x-auto', className]}>
  <table class="data-table">
    <caption class="sr-only">{caption}</caption>
    {@render children()}
  </table>
</div>
