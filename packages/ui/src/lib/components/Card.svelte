<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  type Padding = 'none' | 'md' | 'lg';

  let {
    padding = 'md',
    children,
    class: className = '',
    ...rest
  }: HTMLAttributes<HTMLDivElement> & { padding?: Padding; children: Snippet } = $props();

  const paddingClass: Record<Padding, string> = { none: '', md: 'p-5', lg: 'p-8' };
</script>

<div class={['card', paddingClass[padding], className]} {...rest}>
  {@render children()}
</div>
