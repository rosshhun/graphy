<script lang="ts">
  import type { SymbolDetail as SymbolDetailType } from '../api/client.js';
  import Badge from './Badge.svelte';
  import { langIcon } from '../icons.js';

  let { symbol, onClose, onNavigate }: {
    symbol: SymbolDetailType;
    onClose: () => void;
    onNavigate: (name: string) => void;
  } = $props();
</script>

<div class="detail-panel">
  <div class="detail-header">
    <div class="detail-title">
      <Badge kind={symbol.kind} />
      <h3>{symbol.name}</h3>
    </div>
    <button class="close-btn" onclick={onClose} title="Close">
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M4 4l8 8M12 4l-8 8"/>
      </svg>
    </button>
  </div>

  <div class="detail-meta">
    <span class="meta-item">{symbol.file_path}:{symbol.start_line}</span>
    <span class="meta-item">{symbol.visibility}</span>
    <span class="meta-item meta-lang">
      <span class="meta-lang-icon">{@html langIcon(symbol.language, 14)}</span>
      {symbol.language}
    </span>
  </div>

  {#if symbol.signature}
    <pre class="signature">{symbol.signature}</pre>
  {/if}

  {#if symbol.doc}
    <div class="doc">{symbol.doc}</div>
  {/if}

  {#if symbol.complexity}
    <div class="section">
      <h4>Complexity</h4>
      <div class="metrics-grid">
        <div class="metric">
          <span class="metric-value">{symbol.complexity.cyclomatic}</span>
          <span class="metric-label">Cyclomatic</span>
        </div>
        <div class="metric">
          <span class="metric-value">{symbol.complexity.cognitive}</span>
          <span class="metric-label">Cognitive</span>
        </div>
        <div class="metric">
          <span class="metric-value">{symbol.complexity.sloc}</span>
          <span class="metric-label">SLOC</span>
        </div>
        <div class="metric">
          <span class="metric-value">{symbol.complexity.parameter_count}</span>
          <span class="metric-label">Params</span>
        </div>
        <div class="metric">
          <span class="metric-value">{symbol.complexity.max_nesting_depth}</span>
          <span class="metric-label">Max Depth</span>
        </div>
        <div class="metric">
          <span class="metric-value">{symbol.complexity.loc}</span>
          <span class="metric-label">LOC</span>
        </div>
      </div>
    </div>
  {/if}

  {#if symbol.callers.length > 0}
    <div class="section">
      <h4>Callers ({symbol.callers.length})</h4>
      <ul class="ref-list">
        {#each symbol.callers as ref}
          <li>
            <button class="ref-link" onclick={() => onNavigate(ref.name)}>
              <Badge kind={ref.kind} />
              <span>{ref.name}</span>
            </button>
            <span class="ref-file">{ref.file_path}:{ref.start_line}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if symbol.callees.length > 0}
    <div class="section">
      <h4>Callees ({symbol.callees.length})</h4>
      <ul class="ref-list">
        {#each symbol.callees as ref}
          <li>
            <button class="ref-link" onclick={() => onNavigate(ref.name)}>
              <Badge kind={ref.kind} />
              <span>{ref.name}</span>
            </button>
            <span class="ref-file">{ref.file_path}:{ref.start_line}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if symbol.children.length > 0}
    <div class="section">
      <h4>Children ({symbol.children.length})</h4>
      <ul class="ref-list">
        {#each symbol.children as ref}
          <li>
            <button class="ref-link" onclick={() => onNavigate(ref.name)}>
              <Badge kind={ref.kind} />
              <span>{ref.name}</span>
            </button>
            <span class="ref-file">{ref.file_path}:{ref.start_line}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .detail-panel {
    padding: 16px;
    overflow-y: auto;
    height: 100%;
    font-family: var(--font-mono);
  }
  .detail-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 12px;
  }
  .detail-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .detail-title h3 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text-primary, #e2e8f0);
    word-break: break-all;
  }
  .close-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: 6px;
    color: var(--color-text-secondary, #7c8898);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: color 0.12s, background 0.12s;
  }
  .close-btn svg {
    width: 14px;
    height: 14px;
  }
  .close-btn:hover {
    color: var(--color-text-primary, #e2e8f0);
    background: var(--color-surface-hover, #1c2331);
  }
  .detail-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 12px;
  }
  .meta-item {
    font-size: 11px;
    color: var(--color-text-secondary, #7c8898);
  }
  .meta-lang {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .meta-lang-icon {
    display: flex;
    align-items: center;
  }
  .signature {
    background: var(--color-base, #0d1117);
    border: 1px solid var(--color-border, #252d38);
    border-radius: 4px;
    padding: 8px 12px;
    font-size: 12px;
    color: var(--color-cyan, #22d3ee);
    margin: 0 0 12px;
    overflow-x: auto;
    white-space: pre-wrap;
  }
  .doc {
    font-size: 12px;
    color: var(--color-text-secondary, #7c8898);
    margin-bottom: 12px;
    line-height: 1.5;
    font-family: var(--font-display);
  }
  .section {
    margin-bottom: 16px;
  }
  .section h4 {
    margin: 0 0 8px;
    font-size: 12px;
    color: var(--color-text-secondary, #7c8898);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .metric {
    background: var(--color-base, #0d1117);
    border-radius: 4px;
    padding: 8px;
    text-align: center;
  }
  .metric-value {
    display: block;
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text-primary, #e2e8f0);
  }
  .metric-label {
    display: block;
    font-size: 10px;
    color: var(--color-text-secondary, #7c8898);
    margin-top: 2px;
  }
  .ref-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .ref-list li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 0;
    border-bottom: 1px solid rgba(48, 54, 61, 0.5);
  }
  .ref-link {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    color: var(--color-text-primary, #e2e8f0);
    cursor: pointer;
    font-size: 12px;
    font-family: var(--font-mono);
    padding: 0;
    text-align: left;
  }
  .ref-link:hover { color: var(--color-accent, #58a6ff); }
  .ref-file {
    font-size: 10px;
    color: var(--color-text-secondary, #7c8898);
    padding-left: 28px;
  }
</style>
