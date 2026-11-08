<script lang="ts">
  // A read-only text (a link, a callback URL) with a button that copies it.
  import Check from '@lucide/svelte/icons/check';
  import Copy from '@lucide/svelte/icons/copy';
  import { Button, toast } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import type { FieldControl } from '@cloudit/ui';

  let { value, control }: { value: string; control?: FieldControl } = $props();

  let copied = $state(false);
  let input = $state<HTMLInputElement>();

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // No clipboard access (plain http): select the text so Ctrl+C works.
      input?.select();
      toast.error(t('copy.failed'));
      return;
    }
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }
</script>

<div class="flex gap-2">
  <input
    bind:this={input}
    class="input min-w-0 flex-1 font-mono text-xs"
    readonly
    {value}
    onfocus={(event) => event.currentTarget.select()}
    {...control}
  />
  <Button onclick={copy} aria-label={t('copy.button')}>
    {#if copied}
      <Check size={15} aria-hidden="true" />{t('copy.done')}
    {:else}
      <Copy size={15} aria-hidden="true" />{t('copy.button')}
    {/if}
  </Button>
</div>
