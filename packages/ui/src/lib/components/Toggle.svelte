<script lang="ts">
  // An on/off switch with its label (and optional description) on the left.
  let {
    checked = $bindable(false),
    label,
    description,
    disabled = false,
    onchange,
    class: className = ''
  }: {
    checked?: boolean;
    label: string;
    description?: string;
    disabled?: boolean;
    onchange?: (checked: boolean) => void;
    class?: string;
  } = $props();

  const uid = $props.id();

  function toggle() {
    checked = !checked;
    onchange?.(checked);
  }
</script>

<div class={['flex items-center gap-3.5 py-3', className]}>
  <div class="min-w-0 flex-1">
    <span id="{uid}-label" class="font-semibold">{label}</span>
    {#if description}<p id="{uid}-desc" class="text-xs text-muted">{description}</p>{/if}
  </div>
  <button
    type="button"
    role="switch"
    aria-checked={checked}
    aria-labelledby="{uid}-label"
    aria-describedby={description ? `${uid}-desc` : undefined}
    {disabled}
    onclick={toggle}
    class={[
      'relative h-5 w-9 flex-none cursor-pointer rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-55',
      checked ? 'bg-primary' : 'bg-border-strong'
    ]}
  >
    <span
      class={[
        'absolute top-0.5 left-0.5 size-4 rounded-full bg-on-primary shadow-sm transition-transform',
        checked && 'translate-x-4'
      ]}
    ></span>
  </button>
</div>
