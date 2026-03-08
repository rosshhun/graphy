<script lang="ts">
  import { KIND_COLORS } from '../stores/app.svelte.js';

  let { items, colorByKind }: {
    items: { label: string; value: number }[];
    colorByKind?: boolean;
  } = $props();

  let maxValue = $derived(Math.max(...items.map(i => i.value), 1));
</script>

<div class="bar-chart">
  {#each items as item, i}
    <div class="bar-row" style="animation-delay: {i * 50}ms">
      <span class="bar-label" title={item.label}>{item.label}</span>
      <div class="bar-track">
        <div
          class="bar-fill"
          style="width: {(item.value / maxValue) * 100}%;
                 --bar-color: {colorByKind ? (KIND_COLORS[item.label] || '#6b7280') : `hsl(${210 + i * 18}, 65%, 58%)`}"
        ></div>
      </div>
      <span class="bar-value">{item.value.toLocaleString()}</span>
    </div>
  {/each}
</div>

<style>
  .bar-chart { display: flex; flex-direction: column; gap: 8px; }
  .bar-row {
    display: grid;
    grid-template-columns: 110px 1fr 50px;
    align-items: center;
    gap: 12px;
    animation: slideIn 0.35s ease-out both;
  }
  .bar-label {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--color-text-secondary, #7c8898);
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar-track {
    height: 22px;
    background: var(--color-base, #0d1117);
    border-radius: 2px;
    overflow: hidden;
    position: relative;
  }
  .bar-fill {
    height: 100%;
    border-radius: 2px;
    background: var(--bar-color);
    transition: width 0.6s cubic-bezier(0.22, 1, 0.36, 1);
    min-width: 3px;
    position: relative;
  }
  .bar-fill::after {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(180deg, rgba(255,255,255,0.08) 0%, transparent 100%);
    border-radius: 2px;
  }
  .bar-value {
    font-size: 11px;
    font-family: var(--font-mono);
    font-weight: 500;
    color: var(--color-text-primary, #e2e8f0);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  @keyframes slideIn {
    from { opacity: 0; transform: translateX(-8px); }
    to { opacity: 1; transform: translateX(0); }
  }
</style>
