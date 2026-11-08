<script lang="ts" generics="T extends string">
  // One choice among a few short options, backed by native radio inputs.
  // `track`: grey track with a raised selection (view switchers);
  // `chips`: separate outlined chips (durations, modes).
  let {
    value = $bindable(),
    options,
    name,
    variant = 'track',
    class: className = ''
  }: {
    value: T;
    options: readonly { value: T; label: string }[];
    name: string;
    variant?: 'track' | 'chips';
    class?: string;
  } = $props();

  const focusRing =
    'has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-focus';
</script>

<div
  class={[
    variant === 'track'
      ? 'inline-flex rounded-lg bg-surface-alt p-[3px]'
      : 'flex flex-wrap gap-1.5',
    className
  ]}
>
  {#each options as option (option.value)}
    {@const on = value === option.value}
    <label
      class={[
        'cursor-pointer text-[13px] transition-colors',
        focusRing,
        variant === 'track'
          ? [
              'rounded-md px-3 py-[5px]',
              on ? 'bg-surface font-semibold text-text shadow-sm' : 'text-muted hover:text-text'
            ]
          : [
              'rounded-lg border px-2.5 py-1.5',
              on
                ? 'border-primary bg-primary-soft font-semibold text-primary-fg'
                : 'border-border bg-surface hover:bg-surface-alt'
            ]
      ]}
    >
      <input type="radio" class="sr-only" {name} value={option.value} bind:group={value} />
      {option.label}
    </label>
  {/each}
</div>
