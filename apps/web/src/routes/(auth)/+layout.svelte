<script lang="ts">
  // Pages outside the app shell: one centred card, the language switch in a corner.
  import LanguageSwitch from '#lib/components/LanguageSwitch.svelte';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();
</script>

<div class="relative grid min-h-screen place-items-center p-[30px]">
  <div class="absolute top-4 right-5">
    <LanguageSwitch />
  </div>
  <main class="contents">
    {@render children()}
  </main>
</div>
