<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Variant = 'primary' | 'secondary' | 'danger';

  let {
    variant = 'secondary',
    type = 'button',
    children,
    class: className = '',
    ...rest
  }: HTMLButtonAttributes & { variant?: Variant; children: Snippet } = $props();

  const variantClass: Record<Variant, string> = {
    primary: 'btn-primary',
    secondary: 'btn-secondary',
    danger: 'btn-danger'
  };
</script>

<button {type} class={['btn', variantClass[variant], className]} {...rest}>
  {@render children()}
</button>
