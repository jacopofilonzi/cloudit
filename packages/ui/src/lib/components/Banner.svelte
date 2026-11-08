<script lang="ts" module>
  export type BannerVariant = 'info' | 'success' | 'warning' | 'danger';
</script>

<script lang="ts">
  // An inline message box with an icon, optional title and actions. Danger banners
  // are announced immediately (role="alert"), the others politely.
  import type { Snippet } from 'svelte';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import Info from '@lucide/svelte/icons/info';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

  let {
    variant = 'info',
    title,
    children,
    actions,
    class: className = ''
  }: {
    variant?: BannerVariant;
    title?: string;
    children?: Snippet;
    actions?: Snippet;
    class?: string;
  } = $props();

  const styles: Record<BannerVariant, string> = {
    info: 'border-border bg-surface-alt text-text',
    success: 'border-transparent bg-success-soft text-text',
    warning: 'border-warning-border bg-warning-soft text-text',
    danger: 'border-transparent bg-danger-soft text-text'
  };
  const iconColor: Record<BannerVariant, string> = {
    info: 'text-primary-fg',
    success: 'text-success',
    warning: 'text-warning',
    danger: 'text-danger-fg'
  };
  const icons = { info: Info, success: CircleCheck, warning: TriangleAlert, danger: CircleAlert };
  const Icon = $derived(icons[variant]);
</script>

<div
  role={variant === 'danger' ? 'alert' : 'status'}
  class={['flex items-center gap-3 rounded-xl border px-4 py-3', styles[variant], className]}
>
  <Icon size={18} class={['flex-none', iconColor[variant]]} aria-hidden="true" />
  <div class="min-w-0 flex-1">
    {#if title}<p class="font-semibold">{title}</p>{/if}
    {#if children}
      <div class={title ? 'text-xs text-muted' : ''}>{@render children()}</div>
    {/if}
  </div>
  {#if actions}<div class="flex flex-none gap-2">{@render actions()}</div>{/if}
</div>
