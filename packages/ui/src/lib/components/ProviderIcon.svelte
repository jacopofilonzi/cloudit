<script lang="ts" module>
  // Identity-provider logos from Simple Icons, drawn white on the brand colour (as the
  // mockup's provider badges), so they read in both themes. Only these icons are
  // imported, so the bundle keeps just their paths. Providers without a Simple Icons
  // logo (Microsoft, ZITADEL, Pocket ID, generic OIDC, unknown keys) get a neutral
  // key icon.
  import {
    siAuthelia,
    siAuthentik,
    siDiscord,
    siGithub,
    siGitlab,
    siGoogle,
    siKeycloak,
    type SimpleIcon
  } from 'simple-icons';

  const BRANDS: Record<string, SimpleIcon> = {
    authelia: siAuthelia,
    authentik: siAuthentik,
    discord: siDiscord,
    github: siGithub,
    gitlab: siGitlab,
    google: siGoogle,
    keycloak: siKeycloak
  };

  /** The Simple Icons logo of a provider icon key (`AuthProvider.icon`) or kind. */
  export function brandIcon(key: string): SimpleIcon | undefined {
    return BRANDS[key];
  }
</script>

<script lang="ts">
  import KeyRound from '@lucide/svelte/icons/key-round';

  let {
    icon,
    kind,
    size = 18,
    class: className = ''
  }: {
    /** Icon key of the provider (`AuthProvider.icon`: preset key or `generic`). */
    icon: string;
    /** Fallback when the icon key has no logo (e.g. `generic` on a GitHub provider). */
    kind?: string;
    size?: number;
    class?: string;
  } = $props();

  const brand = $derived(brandIcon(icon) ?? (kind ? brandIcon(kind) : undefined));
</script>

<span
  class={[
    'inline-grid flex-none place-items-center',
    brand ? 'dark:ring-1 dark:ring-border-strong' : 'bg-primary-soft text-primary-fg',
    className
  ]}
  style:width="{size}px"
  style:height="{size}px"
  style:border-radius="{Math.max(3, Math.round(size * 0.22))}px"
  style:background-color={brand ? `#${brand.hex}` : undefined}
  aria-hidden="true"
>
  {#if brand}
    <svg viewBox="0 0 24 24" width={Math.round(size * 0.62)} height={Math.round(size * 0.62)}>
      <path d={brand.path} fill="white" />
    </svg>
  {:else}
    <KeyRound size={Math.round(size * 0.66)} />
  {/if}
</span>
