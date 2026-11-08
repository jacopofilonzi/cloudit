<script lang="ts">
  // The desktop app's sign-in (query contract in `#lib/device.ts`): the signed-in
  // user names the device, picks how long it stays connected and allows it, or
  // cancels. Both answers send the browser back to the app's loopback address.
  import Laptop from '@lucide/svelte/icons/laptop';
  import { page } from '$app/state';
  import type { TokenTtl } from '@cloudit/api-types';
  import { Banner, Button, Field, Input, Segmented } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { authorizeDevice } from '#lib/api.ts';
  import AuthCard from '#lib/components/AuthCard.svelte';
  import { cancelUrl, parseDeviceRequest } from '#lib/device.ts';
  import { formError } from '#lib/errors.ts';
  import { osName } from '#lib/fields.ts';
  import { session } from '#lib/session.svelte.ts';

  const request = parseDeviceRequest(page.url.searchParams);

  let name = $state(request?.device_name ?? '');
  let ttl = $state<TokenTtl>('30d');
  let busy = $state<'allow' | 'cancel' | null>(null);
  let nameInvalid = $state(false);
  let formMessage = $state<string | null>(null);

  const durations = $derived(
    (['1h', '1d', '7d', '30d', '1y', 'forever'] as const).map((value) => ({
      value,
      label: t(`device.ttl_${value}`)
    }))
  );

  async function allow(event: SubmitEvent) {
    event.preventDefault();
    if (!request) return;
    busy = 'allow';
    nameInvalid = false;
    formMessage = null;
    try {
      const { redirect_url } = await authorizeDevice({
        ...request,
        device_name: name.trim(),
        os: request.os || 'unknown',
        token_ttl: ttl
      });
      window.location.assign(redirect_url);
    } catch (err) {
      const { field, form } = formError(err, ['device_name'] as const);
      nameInvalid = field === 'device_name';
      formMessage = form ?? null;
      busy = null;
    }
  }

  function cancel() {
    if (!request) return;
    busy = 'cancel';
    window.location.assign(cancelUrl(request));
  }
</script>

<svelte:head><title>{t('device.title')}</title></svelte:head>

{#if !request}
  <AuthCard title={t('device.invalid_title')}>
    <Banner variant="danger" class="mt-5">{t('device.invalid_body')}</Banner>
  </AuthCard>
{:else}
  <AuthCard wide title={t('device.title')} subtitle={t('device.subtitle')}>
    <div class="my-4 flex items-center gap-3 rounded-[10px] bg-surface-alt p-3.5">
      <span class="icon-tile bg-surface text-muted"><Laptop size={18} aria-hidden="true" /></span>
      <div class="min-w-0">
        <b class="block truncate">{name.trim() || t('device.unnamed')}</b>
        <span class="text-xs text-muted">
          {request.os ? osName(request.os) : t('device.unknown_os')} · {t('device.requested_now')}
        </span>
      </div>
    </div>

    {#if formMessage}<Banner variant="danger" class="mb-1">{formMessage}</Banner>{/if}

    <form onsubmit={allow}>
      <Field
        label={t('device.name')}
        error={nameInvalid ? t('device.name_invalid') : undefined}
        class="mt-0"
      >
        {#snippet children(control)}
          <Input {...control} required maxlength={100} autocomplete="off" bind:value={name} />
        {/snippet}
      </Field>
      <Field label={t('device.duration')} group>
        <Segmented variant="chips" name="device-ttl" options={durations} bind:value={ttl} />
      </Field>
      {#if ttl === 'forever'}
        <Banner variant="warning" class="mt-3">{t('device.forever_hint')}</Banner>
      {:else}
        <p class="hint">{t('device.duration_hint')}</p>
      {/if}

      <div class="mt-5 flex gap-2">
        <Button
          type="submit"
          variant="primary"
          size="lg"
          class="flex-1"
          loading={busy === 'allow'}
          disabled={busy !== null}
        >
          {t('device.allow')}
        </Button>
        <Button size="lg" class="px-[18px]" disabled={busy !== null} onclick={cancel}>
          {t('common.cancel')}
        </Button>
      </div>
    </form>
    <p class="hint mt-3 text-center">
      {t('device.signed_in_as', { email: session.user?.email ?? '' })}
      {t('device.not_you')}
    </p>
  </AuthCard>
{/if}
