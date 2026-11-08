<script lang="ts" module>
  export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-solid';
  export type ButtonSize = 'sm' | 'md' | 'lg';
</script>

<script lang="ts">
  // A button, or a link styled as one when `href` is set. `loading` shows a spinner
  // and disables it (aria-busy), keeping its width.
  import type { Snippet } from 'svelte';
  import type { HTMLAnchorAttributes, HTMLButtonAttributes } from 'svelte/elements';
  import Spinner from './Spinner.svelte';

  type Common = {
    variant?: ButtonVariant;
    size?: ButtonSize;
    /** Full width. */
    block?: boolean;
    loading?: boolean;
    children: Snippet;
  };

  type Props = Common &
    (
      | (HTMLButtonAttributes & { href?: undefined })
      | (HTMLAnchorAttributes & { href: string; disabled?: boolean })
    );

  let {
    variant = 'secondary',
    size = 'md',
    block = false,
    loading = false,
    children,
    class: className = '',
    ...rest
  }: Props = $props();

  const variantClass: Record<ButtonVariant, string> = {
    primary: 'btn-primary',
    secondary: 'btn-secondary',
    ghost: 'btn-ghost',
    danger: 'btn-danger',
    'danger-solid': 'btn-danger-solid'
  };
  const sizeClass: Record<ButtonSize, string> = { sm: 'btn-sm', md: '', lg: 'btn-lg' };

  const classes = $derived([
    'btn',
    variantClass[variant],
    sizeClass[size],
    block && 'w-full',
    className
  ]);
</script>

{#if rest.href !== undefined}
  {@const { disabled, ...link } = rest as HTMLAnchorAttributes & { disabled?: boolean }}
  <a class={classes} aria-disabled={disabled || undefined} {...link}>
    {@render children()}
  </a>
{:else}
  {@const { type = 'button', disabled, ...button } = rest as HTMLButtonAttributes}
  <button
    {type}
    class={classes}
    disabled={disabled || loading}
    aria-busy={loading || undefined}
    {...button}
  >
    {#if loading}<Spinner size={14} />{/if}
    {@render children()}
  </button>
{/if}
