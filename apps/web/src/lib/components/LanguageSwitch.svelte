<script lang="ts">
  import Languages from '@lucide/svelte/icons/languages';
  import { LANGUAGES, getLang, isLang, t } from '@cloudit/ui/i18n';
  import { changeLang } from '#lib/lang.ts';

  const uid = $props.id();

  function onchange(event: Event & { currentTarget: HTMLSelectElement }) {
    const value = event.currentTarget.value;
    if (isLang(value)) changeLang(value);
  }
</script>

<div class="inline-flex items-center gap-1.5 text-xs text-muted">
  <Languages size={14} aria-hidden="true" />
  <label for="{uid}-lang" class="sr-only">{t('language.label')}</label>
  <select
    id="{uid}-lang"
    class="cursor-pointer rounded-md bg-transparent py-1 pr-1 text-xs text-muted hover:text-text"
    value={getLang()}
    {onchange}
  >
    {#each LANGUAGES as lang (lang)}
      <option value={lang}>{t(`language.${lang}`)}</option>
    {/each}
  </select>
</div>
