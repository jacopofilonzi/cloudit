<script lang="ts">
  // "Continue with …" buttons, one per sign-in provider, in the given order.
  import { Button, ProviderIcon } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';

  interface Provider {
    id: string;
    kind: string;
    display_name: string;
    icon: string;
  }

  let {
    providers,
    onstart,
    label = (name: string) => t('auth.continue_with', { provider: name })
  }: {
    providers: readonly Provider[];
    /** Starts the sign-in; the page navigates away on success. */
    onstart: (provider: Provider) => Promise<void>;
    label?: (name: string) => string;
  } = $props();

  let busy = $state<string | null>(null);

  async function start(provider: Provider) {
    busy = provider.id;
    try {
      await onstart(provider);
    } finally {
      busy = null;
    }
  }
</script>

<div class="flex flex-col gap-2">
  {#each providers as provider (provider.id)}
    <Button
      block
      class="py-[9px]"
      loading={busy === provider.id}
      disabled={busy !== null && busy !== provider.id}
      onclick={() => start(provider)}
    >
      {#if busy !== provider.id}<ProviderIcon icon={provider.icon} kind={provider.kind} />{/if}
      {label(provider.display_name)}
    </Button>
  {/each}
</div>
