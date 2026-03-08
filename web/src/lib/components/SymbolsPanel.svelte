<script lang="ts">
  import type { FileSymbolsResponse, FileSymbol } from '../api/client.js';
  import { kindIcon } from '../icons.js';
  import { KIND_COLORS } from '../stores/app.svelte.js';

  let {
    data,
    onSymbolClick,
    activeSymbol = null,
  }: {
    data: FileSymbolsResponse;
    onSymbolClick?: (symbol: FileSymbol) => void;
    activeSymbol?: string | null;
  } = $props();

  let filterQuery = $state('');

  let filteredSymbols = $derived.by(() => {
    if (!filterQuery) return data.symbols;
    const q = filterQuery.toLowerCase();
    return data.symbols.filter(s =>
      s.name.toLowerCase().includes(q) ||
      s.children.some(c => c.name.toLowerCase().includes(q))
    );
  });
</script>

<div class="symbols-panel">
  <div class="sp-header">
    <svg class="sp-icon" width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
      <path d="M9.07694 1.37855C8.38373 1.11193 7.61627 1.11193 6.92306 1.37855L1.96153 3.28683C1.38224 3.50964 1 4.06619 1 4.68685V11.3133C1 11.9339 1.38224 12.4905 1.96153 12.7133L6.92306 14.6216C7.61627 14.8882 8.38373 14.8882 9.07694 14.6216L14.0385 12.7133C14.6178 12.4905 15 11.9339 15 11.3133V4.68685C15 4.06619 14.6178 3.50964 14.0385 3.28683L9.07694 1.37855Z"/>
    </svg>
    <span class="sp-title">Symbols</span>
    <span class="sp-count">{data.symbol_count}</span>
  </div>

  <div class="sp-search">
    <input
      type="text"
      placeholder="Find symbols"
      bind:value={filterQuery}
    />
  </div>

  <div class="sp-tree">
    {#each filteredSymbols as sym}
      <button
        class="sp-item"
        class:sp-active={activeSymbol === sym.name}
        onclick={() => onSymbolClick?.(sym)}
      >
        <span class="sp-kind-icon" style="color:{KIND_COLORS[sym.kind] || '#6b7280'}">{@html kindIcon(sym.kind, 14)}</span>
        <span class="sp-name">{sym.name}</span>
      </button>
      {#if sym.children.length > 0}
        <div class="sp-children">
          {#each sym.children as child}
            <button
              class="sp-item sp-child"
              class:sp-active={activeSymbol === child.name}
              onclick={() => onSymbolClick?.(child)}
            >
              <span class="sp-kind-icon" style="color:{KIND_COLORS[child.kind] || '#6b7280'}">{@html kindIcon(child.kind, 14)}</span>
              <span class="sp-name">{child.name}</span>
            </button>
          {/each}
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .symbols-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .sp-header {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .sp-icon {
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .sp-title {
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text-secondary);
    letter-spacing: 0.2px;
    flex: 1;
  }

  .sp-count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    background: var(--color-surface);
    border-radius: 10px;
    padding: 1px 8px;
    flex-shrink: 0;
  }

  .sp-search {
    padding: 8px 10px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }

  .sp-search input {
    width: 100%;
    height: 28px;
    padding: 0 10px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    color: var(--color-text-primary);
    font-family: var(--font-display);
    font-size: 12px;
    outline: none;
    transition: border-color 0.15s;
  }

  .sp-search input:focus {
    border-color: var(--sg-theme-border-color-focus);
  }

  .sp-search input::placeholder {
    color: var(--color-text-muted);
  }

  .sp-tree {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .sp-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 3px 14px;
    background: none;
    border: none;
    cursor: pointer;
    text-align: left;
    transition: background 0.08s;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--color-text-primary);
    height: 26px;
  }

  .sp-item:hover {
    background: var(--color-surface-hover);
  }

  .sp-item.sp-active {
    background: var(--color-accent-dim);
  }

  .sp-child {
    padding-left: 32px;
  }

  .sp-kind-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }

  .sp-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-children {
    border-left: 1px solid var(--color-border);
    margin-left: 20px;
  }
</style>
