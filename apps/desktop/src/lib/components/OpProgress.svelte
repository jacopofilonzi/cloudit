<script lang="ts">
  // What a running operation is doing: the phase in words and, when the total is known,
  // a bar. Without a report yet it says the operation is starting.
  import type { Progress } from '@cloudit/api-types';
  import { Spinner } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { phaseFraction, phaseText } from '#lib/progress.ts';

  let { progress }: { progress: Progress | undefined } = $props();

  const fraction = $derived(progress ? phaseFraction(progress) : null);
</script>

<div class="flex items-center gap-2 text-xs" role="status" aria-live="polite">
  <Spinner size={14} />
  <span class="min-w-0 flex-1 truncate">
    {progress ? phaseText(progress, getLang()) : t('desktop.folders.checking')}
  </span>
</div>
{#if fraction !== null}
  <div
    class="mt-1.5 h-1.5 overflow-hidden rounded-full bg-border"
    role="progressbar"
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(fraction * 100)}
  >
    <i class="block h-full bg-primary transition-[width]" style:width="{fraction * 100}%"></i>
  </div>
{/if}
