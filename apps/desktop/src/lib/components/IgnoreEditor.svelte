<script lang="ts">
  // Edits a list of ignore patterns as chips. The parent owns the list and decides when
  // it is saved (`onchange` gets the new list).
  import X from '@lucide/svelte/icons/x';
  import { Button, Input } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';

  const SUGGESTION = 'session.lock';

  let {
    patterns,
    onchange,
    disabled = false,
    suggest = true
  }: {
    patterns: readonly string[];
    onchange: (patterns: string[]) => void;
    disabled?: boolean;
    /** Offer to ignore `session.lock` (Minecraft worlds). */
    suggest?: boolean;
  } = $props();

  let draft = $state('');

  function add(pattern: string) {
    const clean = pattern.trim();
    if (clean && !patterns.includes(clean)) onchange([...patterns, clean]);
    draft = '';
  }

  function remove(pattern: string) {
    onchange(patterns.filter((p) => p !== pattern));
  }
</script>

<div>
  <div class="flex flex-wrap gap-1.5">
    {#each patterns as pattern (pattern)}
      <span
        class="inline-flex items-center gap-1 rounded-md bg-surface-alt py-0.5 pr-1 pl-2 font-mono text-[11.5px]"
      >
        {pattern}
        <button
          type="button"
          class="grid size-4 place-items-center rounded text-muted hover:bg-border hover:text-text"
          aria-label={t('desktop.fs.ignore_remove', { pattern })}
          {disabled}
          onclick={() => remove(pattern)}
        >
          <X size={12} aria-hidden="true" />
        </button>
      </span>
    {:else}
      <span class="text-xs text-muted">{t('desktop.fs.ignore_empty')}</span>
    {/each}
  </div>
  <form
    class="mt-2 flex gap-1.5"
    onsubmit={(event) => {
      event.preventDefault();
      add(draft);
    }}
  >
    <Input bind:value={draft} placeholder={t('desktop.fs.ignore_placeholder')} {disabled} />
    <Button type="submit" variant="secondary" disabled={disabled || draft.trim() === ''}>
      {t('desktop.fs.ignore_add')}
    </Button>
  </form>
  {#if suggest && !patterns.includes(SUGGESTION)}
    <button
      type="button"
      class="mt-2 text-xs text-primary-fg hover:underline"
      {disabled}
      onclick={() => add(SUGGESTION)}
    >
      + {t('desktop.fs.ignore_suggest')}
    </button>
  {/if}
  <p class="hint">{t('desktop.fs.ignore_hint')}</p>
</div>
