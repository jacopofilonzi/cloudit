<script lang="ts" generics="T extends string">
  // Underlined tab bar. With `href` on every item it is navigation (links, the
  // current one marked `aria-current`); otherwise a tablist driving `value`
  // (arrow keys move between tabs). The panel is the caller's markup.
  type Item = { value: T; label: string; badge?: number; href?: string };

  let {
    value = $bindable(),
    items,
    label,
    class: className = ''
  }: { value: T; items: readonly Item[]; label: string; class?: string } = $props();

  const isNav = $derived(items.every((item) => item.href !== undefined));
  const buttons: HTMLButtonElement[] = [];

  function onkeydown(event: KeyboardEvent, index: number) {
    const delta = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const next = (index + delta + items.length) % items.length;
    const item = items[next];
    if (!item) return;
    value = item.value;
    buttons[next]?.focus();
  }

  const tabClass = (on: boolean) => [
    'flex items-center gap-1.5 border-b-2 py-2.5 font-semibold no-underline transition-colors',
    on ? 'border-primary text-primary-fg' : 'border-transparent text-muted hover:text-text'
  ];
</script>

{#snippet content(item: Item)}
  {item.label}
  {#if item.badge}
    <span class="rounded-full bg-danger px-[7px] text-[11px] text-on-danger">{item.badge}</span>
  {/if}
{/snippet}

{#if isNav}
  <nav aria-label={label} class={['mb-[18px] flex gap-[22px] border-b border-border', className]}>
    {#each items as item (item.value)}
      <a
        href={item.href}
        class={tabClass(item.value === value)}
        aria-current={item.value === value ? 'page' : undefined}
      >
        {@render content(item)}
      </a>
    {/each}
  </nav>
{:else}
  <div
    role="tablist"
    aria-label={label}
    class={['mb-[18px] flex gap-[22px] border-b border-border', className]}
  >
    {#each items as item, index (item.value)}
      <button
        bind:this={buttons[index]}
        type="button"
        role="tab"
        aria-selected={item.value === value}
        tabindex={item.value === value ? 0 : -1}
        class={['cursor-pointer', tabClass(item.value === value)]}
        onclick={() => (value = item.value)}
        onkeydown={(event) => onkeydown(event, index)}
      >
        {@render content(item)}
      </button>
    {/each}
  </div>
{/if}
