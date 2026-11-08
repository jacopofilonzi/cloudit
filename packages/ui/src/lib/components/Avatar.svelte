<script lang="ts">
  // Initials on a colour picked from the user's stable key (id, else the name).
  import { avatarColorIndex, initials } from '../avatar';

  let {
    name,
    key,
    size = 30,
    ring = false,
    class: className = ''
  }: {
    name: string;
    key?: string;
    size?: number;
    /** Surface-coloured border, for overlapping groups. */
    ring?: boolean;
    class?: string;
  } = $props();

  const color = $derived(`var(--cl-avatar-${avatarColorIndex(key ?? name)})`);
</script>

<span
  class={[
    'inline-grid flex-none place-items-center rounded-full font-bold text-on-primary select-none',
    ring && 'border-2 border-surface',
    className
  ]}
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.4)}px"
  style:background-color={color}
  title={name}
  aria-hidden="true">{initials(name)}</span
>
