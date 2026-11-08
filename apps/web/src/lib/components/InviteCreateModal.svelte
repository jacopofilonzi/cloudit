<script lang="ts">
  // New invite: anyone with the link, or only one email; how long it works.
  import type { TokenTtl } from '@cloudit/api-types';
  import { Banner, Button, Field, Input, Modal, Segmented } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { errorText, formError } from '#lib/errors.ts';
  import { isEmail } from '#lib/fields.ts';

  type Duration = Exclude<TokenTtl, 'forever'>;

  let {
    open = $bindable(false),
    oncreate
  }: { open?: boolean; oncreate: (email: string | undefined, ttl: Duration) => Promise<void> } =
    $props();

  let email = $state('');
  let ttl = $state<Duration>('7d');
  let busy = $state(false);
  let formMessage = $state<string | null>(null);
  let emailError = $state<string | null>(null);

  $effect(() => {
    if (open) {
      email = '';
      ttl = '7d';
      formMessage = emailError = null;
    }
  });

  const uid = $props.id();

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    formMessage = emailError = null;
    const bound = email.trim();
    if (bound && !isEmail(bound)) return void (emailError = t('field.email_invalid'));
    busy = true;
    try {
      await oncreate(bound || undefined, ttl);
      open = false;
    } catch (err) {
      const failure = formError(err, ['email']);
      if (failure.field) emailError = t('field.email_invalid');
      else formMessage = failure.form ?? errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  bind:open
  title={t('admin.invite_new')}
  description={t('admin.invite_new_desc')}
  dismissible={!busy}
>
  <form id="invite-{uid}" onsubmit={submit} class="flex flex-col">
    {#if formMessage}<Banner variant="danger">{formMessage}</Banner>{/if}
    <Field
      label={t('admin.invite_email')}
      hint={t('admin.invite_email_hint')}
      error={emailError ?? undefined}
    >
      {#snippet children(control)}
        <Input {...control} type="email" bind:value={email} autocomplete="off" />
      {/snippet}
    </Field>
    <Field label={t('admin.invite_duration')} group>
      <Segmented
        name="invite-ttl-{uid}"
        variant="chips"
        bind:value={ttl}
        options={[
          { value: '1h', label: t('device.ttl_1h') },
          { value: '1d', label: t('device.ttl_1d') },
          { value: '7d', label: t('device.ttl_7d') },
          { value: '30d', label: t('device.ttl_30d') },
          { value: '1y', label: t('device.ttl_1y') }
        ]}
      />
    </Field>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>{t('common.cancel')}</Button>
    <Button variant="primary" type="submit" form="invite-{uid}" loading={busy}>
      {t('common.create')}
    </Button>
  {/snippet}
</Modal>
