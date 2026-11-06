<script lang="ts">
  import '../app.css';
  import { LANGUAGES, getLang, isLang, t } from '@cloudit/ui/i18n';
  import { changeLang, initLang } from '#lib/lang.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  initLang();

  $effect(() => {
    document.documentElement.lang = getLang();
  });

  function onLangChange(event: Event & { currentTarget: HTMLSelectElement }) {
    const value = event.currentTarget.value;
    if (isLang(value)) changeLang(value);
  }
</script>

<svelte:head>
  <title>{t('app.name')}</title>
</svelte:head>

<div class="mx-auto flex min-h-screen max-w-3xl flex-col gap-6 p-6">
  <header class="flex items-center justify-between gap-4">
    <div>
      <h1 class="text-2xl font-semibold">{t('app.name')}</h1>
      <p class="text-sm text-muted">{t('app.tagline')}</p>
    </div>
    <label class="flex items-center gap-2 text-sm text-muted">
      {t('language.label')}
      <select
        class="rounded-md border border-border bg-surface px-2 py-1 text-text"
        value={getLang()}
        onchange={onLangChange}
      >
        {#each LANGUAGES as lang (lang)}
          <option value={lang}>{t(`language.${lang}`)}</option>
        {/each}
      </select>
    </label>
  </header>

  <main>
    {@render children()}
  </main>
</div>
