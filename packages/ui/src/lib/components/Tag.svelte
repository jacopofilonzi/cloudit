<script lang="ts" module>
  export type TagVariant = 'primary' | 'success' | 'warning' | 'danger' | 'neutral';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    variant = 'primary',
    children,
    class: className = ''
  }: { variant?: TagVariant; children: Snippet; class?: string } = $props();

  const variantClass: Record<TagVariant, string> = {
    primary: 'bg-primary-soft text-primary-fg',
    success: 'bg-success-soft text-success',
    warning: 'bg-warning-soft text-warning',
    danger: 'bg-danger-soft text-danger-fg',
    neutral: 'bg-surface-alt text-muted'
  };
</script>

<span class={['tag', variantClass[variant], className]}>{@render children()}</span>
