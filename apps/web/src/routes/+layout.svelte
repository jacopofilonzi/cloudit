<script lang="ts">
  // Root: global CSS, language, theme, the session and the route guard. Pages render
  // only once the session is known and the guard has nothing to redirect.
  import '../app.css';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { Banner, Button, ConfirmHost, Spinner, Toaster, initTheme } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import { APP_ROOT } from '#lib/api.ts';
  import { errorText } from '#lib/errors.ts';
  import { guard } from '#lib/guard.ts';
  import { initLang } from '#lib/lang.ts';
  import { session } from '#lib/session.svelte.ts';
  import type { LayoutProps } from './$types';

  let { children }: LayoutProps = $props();

  initLang();
  initTheme();
  void session.load();

  /** The path without the app's base path, starting with `/`. */
  const route = $derived(`/${page.url.pathname.slice(APP_ROOT.length)}`);
  const target = $derived(
    session.status === 'ready'
      ? guard(route, page.url.search, {
          setupRequired: session.setupRequired,
          signedIn: session.signedIn
        })
      : null
  );

  $effect(() => {
    document.documentElement.lang = getLang();
  });

  $effect(() => {
    // Guard targets are dynamic in-app paths (`return_to`), so typed `resolve()` can't
    // take them; APP_ROOT is `resolve('')` (the base path).
    // eslint-disable-next-line svelte/no-navigation-without-resolve
    if (target) void goto(APP_ROOT + target.slice(1), { replaceState: true });
  });
</script>

<svelte:head>
  <title>{t('app.name')}</title>
</svelte:head>

{#if session.status === 'error'}
  <div class="grid min-h-screen place-items-center p-8">
    <div class="w-full max-w-[400px]">
      <Banner variant="danger" title={t('app.load_failed')}>
        {errorText(session.error)}
        {#snippet actions()}
          <Button size="sm" onclick={() => session.load()}>{t('common.retry')}</Button>
        {/snippet}
      </Banner>
    </div>
  </div>
{:else if session.status === 'loading' || target}
  <div class="grid min-h-screen place-items-center">
    <Spinner size={22} label={t('common.loading')} />
  </div>
{:else}
  {@render children()}
{/if}

<Toaster />
<ConfirmHost />
