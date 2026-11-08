<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  let {
    value = $bindable(''),
    class: className = '',
    ...rest
  }: Omit<HTMLInputAttributes, 'value'> & { value?: string } = $props();
</script>

<input class={['input', className]} bind:value {...rest} />
