<script lang="ts">
  // Shows the toasts of `toast.svelte.ts`; mount once in the root layout.
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import Info from '@lucide/svelte/icons/info';
  import X from '@lucide/svelte/icons/x';
  import { t } from '../i18n/i18n.svelte';
  import { dismissToast, getToasts } from '../toast.svelte';

  const icons = { success: CircleCheck, info: Info, error: CircleAlert };
  const colors = { success: 'text-success', info: 'text-primary-fg', error: 'text-danger-fg' };
</script>

<div
  class="pointer-events-none fixed right-4 bottom-4 z-50 flex w-[min(360px,calc(100%-2rem))] flex-col gap-2"
  aria-live="polite"
  aria-relevant="additions"
>
  {#each getToasts() as item (item.id)}
    {@const Icon = icons[item.kind]}
    <div
      class="card pointer-events-auto flex items-start gap-2.5 px-4 py-3 shadow-[0_8px_30px_var(--cl-shadow)]"
      role={item.kind === 'error' ? 'alert' : undefined}
    >
      <Icon size={18} class={['mt-px flex-none', colors[item.kind]]} aria-hidden="true" />
      <p class="min-w-0 flex-1">{item.message}</p>
      <button
        type="button"
        class="btn btn-ghost btn-sm -my-1 -mr-2"
        aria-label={t('common.dismiss')}
        onclick={() => dismissToast(item.id)}
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  {/each}
</div>
