<script lang="ts">
  // `/`: opens the scope last visited (else the first one); with none, invites the
  // user to create the first.
  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import { Button, EmptyState, Spinner } from '@cloudit/ui';
  import { t } from '@cloudit/ui/i18n';
  import { app } from '#lib/app.svelte.ts';
  import ScopeCreateModal from '#lib/components/ScopeCreateModal.svelte';

  let creating = $state(false);

  function lastScope(): string | null {
    try {
      return localStorage.getItem('cloudit.lastScope');
    } catch {
      return null;
    }
  }

  const target = $derived.by(() => {
    const scopes = app.scopes;
    if (!scopes || scopes.length === 0) return undefined;
    const last = lastScope();
    return scopes.find((scope) => scope.id === last) ?? scopes[0];
  });

  $effect(() => {
    if (target) void goto(resolve('/(app)/scopes/[id]', { id: target.id }), { replace: true });
  });
</script>

<svelte:head><title>{t('app.name')}</title></svelte:head>

{#if app.scopes?.length === 0}
  <EmptyState icon={FolderPlus} title={t('home.empty_title')} description={t('home.empty_body')}>
    {#snippet action()}
      <Button variant="primary" onclick={() => (creating = true)}>{t('home.create')}</Button>
    {/snippet}
  </EmptyState>
  <ScopeCreateModal bind:open={creating} />
{:else}
  <div class="grid place-items-center py-20"><Spinner size={22} label={t('common.loading')} /></div>
{/if}
