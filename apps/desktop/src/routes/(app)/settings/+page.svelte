<script lang="ts">
  // App settings: the account of this device, language and theme, what is coming, the
  // versions and the log folder.
  import {
    Avatar,
    Banner,
    Button,
    Segmented,
    Toggle,
    confirmAction,
    formatDate,
    getThemeMode,
    setThemeMode,
    THEME_MODES,
    type ThemeMode
  } from '@cloudit/ui';
  import { LANGUAGES, getLang, t, type Lang } from '@cloudit/ui/i18n';
  import { reportError } from '#lib/actions.ts';
  import * as api from '#lib/api.ts';
  import { app } from '#lib/app.svelte.ts';
  import { changeLang } from '#lib/lang.ts';

  const account = $derived(app.account);
  let lang = $state<Lang>(getLang());
  let theme = $state<ThemeMode>(getThemeMode());

  $effect(() => {
    if (lang !== getLang()) changeLang(lang);
  });
  $effect(() => {
    if (theme !== getThemeMode()) setThemeMode(theme);
  });

  const themeLabels: Record<ThemeMode, () => string> = {
    system: () => t('desktop.settings.theme_system'),
    light: () => t('desktop.settings.theme_light'),
    dark: () => t('desktop.settings.theme_dark')
  };
  const themeOptions = $derived(
    THEME_MODES.map((mode) => ({ value: mode, label: themeLabels[mode]() }))
  );
  const langOptions = LANGUAGES.map((l) => ({
    value: l,
    label: l === 'en' ? 'English' : 'Italiano'
  }));

  async function signOut() {
    const ok = await confirmAction({
      title: t('desktop.settings.sign_out_title'),
      message: t('desktop.settings.sign_out_body'),
      confirmLabel: t('desktop.settings.sign_out'),
      danger: true
    });
    if (!ok) return;
    try {
      await api.signOut();
      app.signedOut();
    } catch (error) {
      reportError(error);
    }
  }
</script>

<div class="mx-auto max-w-[620px]">
  <h1 class="mb-3 !text-[19px]">{t('desktop.settings.title')}</h1>

  <div class="card p-[18px]">
    <h2>{t('desktop.settings.account')}</h2>
    {#if account}
      <div class="mt-3 flex items-center gap-3">
        <Avatar name={account.display_name} key={account.user_id} size={34} />
        <div class="min-w-0">
          <b class="block truncate">
            {t('desktop.settings.signed_as', { name: account.display_name, email: account.email })}
          </b>
          <span class="block truncate text-xs text-muted">
            {account.server_url} · {t('desktop.settings.device')}: <b>{account.device_name}</b>
          </span>
          <span class="block text-xs text-muted">
            {account.token_expires_at
              ? t('desktop.settings.token_until', {
                  date: formatDate(account.token_expires_at, getLang())
                })
              : t('desktop.settings.token_forever')}
          </span>
        </div>
        <Button variant="danger" size="sm" class="ms-auto" onclick={signOut}>
          {t('desktop.settings.sign_out')}
        </Button>
      </div>
      {#if app.tokenWarning}
        <Banner variant="warning" class="mt-3">{t('desktop.settings.token_warning')}</Banner>
      {/if}
    {/if}

    <h2 class="mt-5">{t('desktop.settings.app')}</h2>
    <div class="mt-2 divide-y divide-border">
      <div class="flex items-center gap-3 py-3">
        <b>{t('desktop.settings.language')}</b>
        <Segmented class="ms-auto" name="language" bind:value={lang} options={langOptions} />
      </div>
      <div class="flex items-center gap-3 py-3">
        <b>{t('desktop.settings.theme')}</b>
        <Segmented class="ms-auto" name="theme" bind:value={theme} options={themeOptions} />
      </div>
      <div class="py-3">
        <Toggle
          checked={false}
          disabled
          label={t('desktop.settings.autostart')}
          description={t('desktop.settings.autostart_hint')}
        />
      </div>
      <div class="py-3">
        <Toggle
          checked={app.autoSync}
          label={t('desktop.settings.auto_sync')}
          description={t('desktop.settings.auto_sync_hint')}
          onchange={(on) => app.setAutoSync(on).catch(reportError)}
        />
      </div>
    </div>

    <h2 class="mt-5">{t('desktop.settings.about')}</h2>
    <div class="mt-2 flex items-center gap-3 text-xs text-muted">
      <span>
        {t('desktop.settings.versions', {
          app: app.appVersion,
          server: app.serverVersion ?? t('desktop.settings.server_unknown')
        })}
      </span>
      <button
        type="button"
        class="ms-auto text-primary-fg hover:underline"
        onclick={() => api.openLogDir().catch(reportError)}
      >
        {t('desktop.settings.log')}
      </button>
    </div>
  </div>
</div>
