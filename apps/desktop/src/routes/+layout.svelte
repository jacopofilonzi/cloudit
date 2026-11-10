<script lang="ts">
  // Root: global CSS, language, theme, the backend session and the route guard. Pages
  // render once the session is known; signed-out users only see /welcome.
  import '../app.css';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
  import { Banner, ConfirmHost, Spinner, Toaster, initTheme } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { commandErrorText } from '#lib/errors.ts';
  import { initLang } from '#lib/lang.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  initLang();
  initTheme();
  onMount(() => {
    void app.init();
    return () => app.dispose();
  });

  const onWelcome = $derived(page.url.pathname.startsWith(resolve('/welcome')));
  const atRoot = $derived(page.url.pathname === resolve('/'));

  $effect(() => {
    const lang = getLang();
    document.documentElement.lang = lang;
    // The tray menu and the notifications are written by the backend in this language.
    api.setUiLanguage(lang).catch((error: unknown) => app.noteError(error));
  });

  $effect(() => {
    if (!app.ready || app.startError) return;
    if (!app.signedIn && !onWelcome) void goto(resolve('/welcome'), { replaceState: true });
    else if (app.signedIn && (onWelcome || atRoot)) {
      void goto(resolve('/(app)/folders'), { replaceState: true });
    }
  });

  const routable = $derived(app.ready && !app.startError && app.signedIn !== onWelcome);
</script>

<svelte:head>
  <title>{t('app.name')}</title>
</svelte:head>

{#if app.startError}
  <div class="grid h-screen place-items-center p-8">
    <div class="w-full max-w-[420px]">
      <Banner variant="danger" title={t('desktop.shell.start_failed')}>
        {commandErrorText(app.startError)}
      </Banner>
    </div>
  </div>
{:else if routable}
  {@render children()}
{:else}
  <div class="grid h-screen place-items-center">
    <Spinner size={22} label={t('desktop.shell.loading')} />
  </div>
{/if}

<Toaster />
<ConfirmHost />
