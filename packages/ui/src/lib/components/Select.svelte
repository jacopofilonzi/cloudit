<script lang="ts" generics="T extends string">
  // A native select over string options.
  import type { HTMLSelectAttributes } from 'svelte/elements';

  let {
    value = $bindable(),
    options,
    class: className = '',
    ...rest
  }: Omit<HTMLSelectAttributes, 'value'> & {
    value: T;
    options: readonly { value: T; label: string }[];
  } = $props();
</script>

<select class={['input', className]} bind:value {...rest}>
  {#each options as option (option.value)}
    <option value={option.value}>{option.label}</option>
  {/each}
</select>
