<script lang="ts" module>
  /** Attributes a control inside a `Field` spreads onto itself. */
  export interface FieldControl {
    id: string;
    'aria-describedby': string | undefined;
    'aria-invalid': true | undefined;
  }
</script>

<script lang="ts">
  // Label + control + hint + error. The control is rendered by the `children`
  // snippet, which receives the ids to wire (`<Input {...control} />`). With
  // `group`, the label names a group of controls (radio cards, segmented).
  import type { Snippet } from 'svelte';

  let {
    label,
    hint,
    error,
    group = false,
    children,
    class: className = ''
  }: {
    label: string;
    hint?: string | Snippet;
    error?: string;
    group?: boolean;
    children: Snippet<[FieldControl]>;
    class?: string;
  } = $props();

  const uid = $props.id();
  const id = `field-${uid}`;
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  const labelId = `${id}-label`;

  const control = $derived<FieldControl>({
    id,
    'aria-describedby':
      [hint ? hintId : '', error ? errorId : ''].filter(Boolean).join(' ') || undefined,
    'aria-invalid': error ? true : undefined
  });
</script>

<div class={['mt-3.5', className]}>
  {#if group}
    <span id={labelId} class="field-label">{label}</span>
    <div role="group" aria-labelledby={labelId}>
      {@render children(control)}
    </div>
  {:else}
    <label for={id} class="field-label">{label}</label>
    {@render children(control)}
  {/if}
  {#if error}
    <p id={errorId} class="field-error">{error}</p>
  {/if}
  {#if hint}
    <div id={hintId} class="hint">
      {#if typeof hint === 'string'}{hint}{:else}{@render hint()}{/if}
    </div>
  {/if}
</div>
