<script lang="ts">
  // Shows a one-time link (invite, password reset) to copy. The token is not stored
  // anywhere readable again, so the dialog says so.
  import { Banner, Button, Field, Modal, formatDateTime } from '@cloudit/ui';
  import { getLang, t } from '@cloudit/ui/i18n';
  import CopyField from './CopyField.svelte';

  let {
    open = $bindable(false),
    title,
    description,
    url,
    expiresAt
  }: {
    open?: boolean;
    title: string;
    description?: string;
    url: string;
    expiresAt: string;
  } = $props();
</script>

<Modal bind:open {title} {description}>
  <Field label={t('link.label')}>
    {#snippet children(control)}
      <CopyField value={url} {control} />
    {/snippet}
  </Field>
  <Banner class="mt-4" variant="warning">
    {t('link.shown_once', { when: formatDateTime(expiresAt, getLang()) })}
  </Banner>
  {#snippet footer()}
    <Button variant="primary" onclick={() => (open = false)}>{t('link.done')}</Button>
  {/snippet}
</Modal>
