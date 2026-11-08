<script lang="ts">
  // Overlapping avatars; beyond `max` a "+n" counter. The list of names is the
  // accessible label.
  import Avatar from './Avatar.svelte';

  let {
    people,
    max = 4,
    size = 26,
    class: className = ''
  }: {
    people: readonly { name: string; key?: string }[];
    max?: number;
    size?: number;
    class?: string;
  } = $props();

  const shown = $derived(people.slice(0, max));
  const extra = $derived(people.length - shown.length);
</script>

<span
  class={['inline-flex ps-1.5', className]}
  role="img"
  aria-label={people.map((p) => p.name).join(', ')}
>
  {#each shown as person (person.key ?? person.name)}
    <Avatar name={person.name} key={person.key} {size} ring class="-ms-1.5" />
  {/each}
  {#if extra > 0}
    <span
      class="-ms-1.5 inline-grid place-items-center rounded-full border-2 border-surface bg-surface-alt text-[11px] font-bold text-muted"
      style:width="{size}px"
      style:height="{size}px"
      aria-hidden="true">+{extra}</span
    >
  {/if}
</span>
