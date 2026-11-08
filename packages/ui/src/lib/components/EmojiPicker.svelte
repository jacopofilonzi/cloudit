<script lang="ts">
  // Picks a folder emoji: a grid of common ones plus a free field for any other
  // (paste or the OS emoji keyboard: Win + . / Ctrl+Cmd+Space). `""` = none.
  import Ban from '@lucide/svelte/icons/ban';
  import { FOLDER_EMOJI, isValidEmoji } from '../emoji';
  import { t } from '../i18n/i18n.svelte';

  let {
    value = $bindable(''),
    onselect,
    class: className = ''
  }: { value?: string; onselect?: (emoji: string) => void; class?: string } = $props();

  const uid = $props.id();
  let custom = $state('');
  const customValid = $derived(isValidEmoji(custom));

  function pick(emoji: string) {
    value = emoji;
    onselect?.(emoji);
  }

  function useCustom(event: SubmitEvent) {
    event.preventDefault();
    const emoji = custom.trim();
    if (emoji && customValid) pick(emoji);
  }
</script>

<div class={['w-[300px] max-w-full', className]}>
  <div class="grid grid-cols-8 gap-1" role="group" aria-label={t('emoji.choose')}>
    <button
      type="button"
      class={[
        'grid aspect-square cursor-pointer place-items-center rounded-md text-muted hover:bg-surface-alt',
        value === '' && 'bg-primary-soft text-primary-fg'
      ]}
      aria-pressed={value === ''}
      aria-label={t('emoji.none')}
      title={t('emoji.none')}
      onclick={() => pick('')}
    >
      <Ban size={16} aria-hidden="true" />
    </button>
    {#each FOLDER_EMOJI as emoji (emoji)}
      <button
        type="button"
        class={[
          'grid aspect-square cursor-pointer place-items-center rounded-md text-lg hover:bg-surface-alt',
          value === emoji && 'bg-primary-soft'
        ]}
        aria-pressed={value === emoji}
        onclick={() => pick(emoji)}>{emoji}</button
      >
    {/each}
  </div>
  <form class="mt-3 flex gap-2" onsubmit={useCustom}>
    <label for="{uid}-custom" class="sr-only">{t('emoji.custom')}</label>
    <input
      id="{uid}-custom"
      class="input py-1.5"
      placeholder={t('emoji.custom_placeholder')}
      maxlength={16}
      aria-invalid={customValid ? undefined : true}
      bind:value={custom}
    />
    <button
      type="submit"
      class="btn btn-secondary btn-sm"
      disabled={!custom.trim() || !customValid}
    >
      {t('emoji.use')}
    </button>
  </form>
  {#if !customValid}<p class="field-error">{t('emoji.invalid')}</p>{/if}
</div>
