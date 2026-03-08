<script lang="ts">
  import type { SymbolDetail } from '../api/client.js';
  import Badge from './Badge.svelte';

  let {
    symbol,
    onNavigate,
  }: {
    symbol: SymbolDetail;
    onNavigate: (name: string) => void;
  } = $props();

  type Tab = 'callers' | 'callees' | 'children';
  let activeTab = $state<Tab>('callers');

  let tabs = $derived([
    { key: 'callers' as Tab, label: 'Callers', count: symbol.callers.length },
    { key: 'callees' as Tab, label: 'Callees', count: symbol.callees.length },
    { key: 'children' as Tab, label: 'Children', count: symbol.children.length },
  ].filter(t => t.count > 0));

  let activeRefs = $derived.by(() => {
    if (activeTab === 'callers') return symbol.callers;
    if (activeTab === 'callees') return symbol.callees;
    return symbol.children;
  });

  $effect(() => {
    if (tabs.length > 0 && !tabs.find(t => t.key === activeTab)) {
      activeTab = tabs[0].key;
    }
  });
</script>

{#if tabs.length > 0}
<div class="ref-panel">
  <div class="ref-tabs">
    {#each tabs as tab}
      <button
        class="ref-tab"
        class:active={activeTab === tab.key}
        onclick={() => activeTab = tab.key}
      >
        {tab.label}
        <span class="ref-tab-count">{tab.count}</span>
      </button>
    {/each}
    <div class="ref-tab-spacer"></div>
    <span class="ref-symbol-name">
      <Badge kind={symbol.kind} />
      {symbol.name}
    </span>
  </div>
  <div class="ref-content">
    {#each activeRefs as ref}
      <button class="ref-row" onclick={() => onNavigate(ref.name)}>
        <Badge kind={ref.kind} />
        <span class="ref-name">{ref.name}</span>
        <span class="ref-file">{ref.file_path}:{ref.start_line}</span>
      </button>
    {/each}
  </div>
</div>
{/if}

<style>
  .ref-panel {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--color-border);
    max-height: 200px;
    overflow: hidden;
  }

  .ref-tabs {
    display: flex;
    align-items: center;
    gap: 0;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    padding: 0 8px;
    background: var(--color-surface);
  }

  .ref-tab {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 6px 12px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: color 0.1s, border-color 0.1s;
  }

  .ref-tab:hover {
    color: var(--color-text-primary);
  }

  .ref-tab.active {
    color: var(--color-text-primary);
    border-bottom-color: var(--color-accent);
  }

  .ref-tab-count {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-text-muted);
    background: var(--color-base);
    border-radius: 10px;
    padding: 0 6px;
  }

  .ref-tab-spacer {
    flex: 1;
  }

  .ref-symbol-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .ref-content {
    overflow-y: auto;
    flex: 1;
  }

  .ref-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 14px;
    background: none;
    border: none;
    border-bottom: 1px solid var(--color-border);
    cursor: pointer;
    text-align: left;
    transition: background 0.08s;
  }

  .ref-row:last-child {
    border-bottom: none;
  }

  .ref-row:hover {
    background: var(--color-surface-hover);
  }

  .ref-name {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--color-text-primary);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ref-file {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }
</style>
