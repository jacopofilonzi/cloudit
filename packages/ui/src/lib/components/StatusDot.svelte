<script lang="ts">
  // Coloured dot for a state (online / attention / off). Decorative: pair it with text.
  let { status = 'ok', class: className = '' }: { status?: 'ok' | 'warn' | 'off'; class?: string } =
    $props();

  const statusClass = { ok: 'bg-success', warn: 'bg-warning', off: 'bg-off' };
</script>

<span class={['status-dot', statusClass[status], className]} aria-hidden="true"></span>
