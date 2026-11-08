<script lang="ts" generics="T extends string">
  // A group of selectable cards (title + description) backed by native radio
  // inputs, so arrow keys and screen readers work as for any radio group.
  let {
    value = $bindable(),
    options,
    name,
    columns = 1,
    class: className = ''
  }: {
    value: T;
    options: readonly { value: T; title: string; description?: string }[];
    /** Form name of the radio group (unique on the page). */
    name: string;
    columns?: 1 | 2;
    class?: string;
  } = $props();
</script>

<div
  class={['grid gap-2.5', columns === 2 ? 'grid-cols-1 sm:grid-cols-2' : 'grid-cols-1', className]}
>
  {#each options as option (option.value)}
    {@const on = value === option.value}
    <label
      class={[
        'flex cursor-pointer items-start gap-2.5 rounded-[10px] border px-3 py-2.5 transition-colors has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-focus',
        on ? 'border-primary bg-primary-soft' : 'border-border bg-surface hover:bg-surface-alt'
      ]}
    >
      <input type="radio" class="sr-only" {name} value={option.value} bind:group={value} />
      <span
        class={[
          'mt-0.5 size-4 flex-none rounded-full',
          on ? 'border-[5px] border-primary' : 'border-2 border-border-strong'
        ]}
        aria-hidden="true"
      ></span>
      <span class="min-w-0">
        <span class="block font-semibold">{option.title}</span>
        {#if option.description}
          <span class="block text-xs text-muted">{option.description}</span>
        {/if}
      </span>
    </label>
  {/each}
</div>
