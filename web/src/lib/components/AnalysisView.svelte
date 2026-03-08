<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    type HotspotItem,
    type DeadCodeItem,
    type PatternFinding,
    type ApiSurface,
  } from '../api/client.js';
  import Badge from './Badge.svelte';
  import LoadingPulse from './LoadingPulse.svelte';

  let hotspots = $state<HotspotItem[]>([]);
  let deadCode = $state<DeadCodeItem[]>([]);
  let patterns = $state<PatternFinding[]>([]);
  let apiSurface = $state<ApiSurface | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      const [h, d, p, a] = await Promise.all([
        api.hotspots(20),
        api.deadCode(20),
        api.patterns(20),
        api.apiSurface(),
      ]);
      hotspots = h;
      deadCode = d;
      patterns = p;
      apiSurface = a;
    } catch (e) {
      error = 'Failed to load analysis data. Is the server running?';
      console.error('Failed to load analysis data:', e);
    } finally {
      loading = false;
    }
  });

  let healthScore = $derived.by(() => {
    if (!apiSurface) return 0;
    const hotspotPenalty = Math.min(hotspots.length * 3, 30);
    const deadPenalty = Math.min(deadCode.length * 2, 30);
    const patternPenalty = Math.min(patterns.length * 2, 20);
    return Math.max(0, 100 - hotspotPenalty - deadPenalty - patternPenalty);
  });

  let healthColor = $derived(
    healthScore >= 80 ? 'var(--color-green, #4ade80)' :
    healthScore >= 50 ? 'var(--color-yellow, #fbbf24)' :
    'var(--color-red, #f87171)'
  );

  function severityColor(severity: string): string {
    if (severity === 'warning') return 'var(--color-yellow, #fbbf24)';
    if (severity === 'error' || severity === 'critical') return 'var(--color-red, #f87171)';
    return 'var(--color-accent, #58a6ff)';
  }
</script>

<div class="scroll-area">
  {#if loading}
    <div class="center-state"><LoadingPulse /></div>
  {:else if error}
    <div class="center-state"><p class="error-text">{error}</p></div>
  {:else}
    <div class="container">
      <!-- Page Header -->
      <header class="page-header">
        <h1>Analysis</h1>
        <p>Codebase health, complexity hotspots, and quality issues</p>
      </header>

      <!-- Summary -->
      <div class="summary">
        <div class="summary-primary">
          <span class="score" style="color: {healthColor}">{healthScore}</span>
          <div class="score-meta">
            <span class="score-label">Health Score</span>
            <div class="score-bar">
              <div class="score-bar-fill" style="width: {healthScore}%; background: {healthColor}"></div>
            </div>
          </div>
        </div>
        <div class="summary-divider"></div>
        <div class="summary-stats">
          <div class="summary-stat">
            <span class="summary-val">{hotspots.length}</span>
            <span class="summary-lbl">Hotspots</span>
          </div>
          <div class="summary-stat">
            <span class="summary-val">{deadCode.length}</span>
            <span class="summary-lbl">Dead Code</span>
          </div>
          <div class="summary-stat">
            <span class="summary-val">{patterns.length}</span>
            <span class="summary-lbl">Anti-Patterns</span>
          </div>
        </div>
      </div>

      <!-- Hotspots -->
      {#if hotspots.length > 0}
        <section class="section">
          <div class="section-head">
            <h2>Complexity Hotspots</h2>
            <span class="section-count">{hotspots.length} items</span>
          </div>
          <div class="card">
            <div class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>Symbol</th>
                    <th>Kind</th>
                    <th class="r">Cyclomatic</th>
                    <th class="r">Cognitive</th>
                    <th class="r">LOC</th>
                    <th class="r">Callers</th>
                    <th class="r">Risk</th>
                  </tr>
                </thead>
                <tbody>
                  {#each hotspots as item}
                    <tr>
                      <td>
                        <div class="cell-stack">
                          <span class="cell-name">{item.name}</span>
                          <span class="cell-path">{item.file_path}:{item.start_line}</span>
                        </div>
                      </td>
                      <td><Badge kind={item.kind} /></td>
                      <td class="num">{item.cyclomatic}</td>
                      <td class="num">{item.cognitive}</td>
                      <td class="num">{item.loc}</td>
                      <td class="num">{item.caller_count}</td>
                      <td class="num risk">{item.risk_score.toFixed(1)}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </div>
        </section>
      {/if}

      <!-- Dead Code -->
      {#if deadCode.length > 0}
        <section class="section">
          <div class="section-head">
            <h2>Dead Code</h2>
            <span class="section-count">{deadCode.length} items</span>
          </div>
          <div class="card">
            <div class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>Symbol</th>
                    <th>Kind</th>
                    <th>Visibility</th>
                    <th>Probability</th>
                  </tr>
                </thead>
                <tbody>
                  {#each deadCode as item}
                    <tr>
                      <td>
                        <div class="cell-stack">
                          <span class="cell-name">{item.name}</span>
                          <span class="cell-path">{item.file_path}:{item.start_line}</span>
                        </div>
                      </td>
                      <td><Badge kind={item.kind} /></td>
                      <td><span class="tag">{item.visibility}</span></td>
                      <td>
                        <div class="prob">
                          <div class="prob-track">
                            <div class="prob-fill" style="width: {item.dead_probability * 100}%; background: {item.dead_probability > 0.7 ? 'var(--color-red, #f87171)' : 'var(--color-yellow, #fbbf24)'}"></div>
                          </div>
                          <span class="prob-pct">{(item.dead_probability * 100).toFixed(0)}%</span>
                        </div>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </div>
        </section>
      {/if}

      <!-- Anti-Patterns -->
      {#if patterns.length > 0}
        <section class="section">
          <div class="section-head">
            <h2>Anti-Patterns</h2>
            <span class="section-count">{patterns.length} items</span>
          </div>
          <div class="card">
            {#each patterns as item, i}
              <div class="pattern-row" class:first={i === 0}>
                <div class="pattern-main">
                  <span class="pattern-name">{item.pattern}</span>
                  <span class="pattern-sev" style="color: {severityColor(item.severity)}">{item.severity}</span>
                </div>
                <div class="pattern-meta">
                  <span>{item.symbol_name}</span>
                  <span class="dot">&middot;</span>
                  <span>{item.file_path}:{item.line}</span>
                </div>
                <div class="pattern-detail">{item.detail}</div>
              </div>
            {/each}
          </div>
        </section>
      {/if}

      {#if hotspots.length === 0 && deadCode.length === 0 && patterns.length === 0}
        <section class="section">
          <div class="clean-state">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="clean-icon">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
              <polyline points="22 4 12 14.01 9 11.01"/>
            </svg>
            <span>No issues detected. Codebase looks healthy.</span>
          </div>
        </section>
      {/if}

      <!-- API Surface -->
      {#if apiSurface}
        <section class="section">
          <div class="section-head">
            <h2>API Surface</h2>
          </div>
          <div class="card">
            <div class="api-grid">
              <div class="api-item">
                <span class="api-num" style="color: var(--color-green, #4ade80)">{apiSurface.public.length}</span>
                <span class="api-label">Public</span>
              </div>
              <div class="api-item">
                <span class="api-num" style="color: var(--color-yellow, #fbbf24)">{apiSurface.effectively_internal.length}</span>
                <span class="api-label">Effectively Internal</span>
              </div>
              <div class="api-item">
                <span class="api-num">{apiSurface.internal_count}</span>
                <span class="api-label">Internal</span>
              </div>
              <div class="api-item">
                <span class="api-num">{apiSurface.private_count}</span>
                <span class="api-label">Private</span>
              </div>
            </div>
          </div>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Layout */
  .scroll-area {
    height: 100%;
    overflow-y: auto;
  }
  .container {
    max-width: 1080px;
    margin: 0 auto;
    padding: 32px 40px 64px;
  }
  .center-state {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
  }
  .error-text {
    color: var(--color-text-secondary);
    font-size: 13px;
  }

  /* Page Header */
  .page-header {
    margin-bottom: 28px;
  }
  .page-header h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 20px;
    font-weight: 700;
    color: var(--color-text-primary);
    letter-spacing: -0.3px;
  }
  .page-header p {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--color-text-muted);
  }

  /* Summary */
  .summary {
    display: flex;
    align-items: center;
    gap: 28px;
    padding: 22px 26px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    margin-bottom: 36px;
  }
  .summary-primary {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-shrink: 0;
  }
  .score {
    font-family: var(--font-display);
    font-size: 38px;
    font-weight: 700;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .score-meta {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .score-label {
    font-size: 11px;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .score-bar {
    width: 80px;
    height: 4px;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 2px;
    overflow: hidden;
  }
  .score-bar-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.6s ease-out;
  }
  .summary-divider {
    width: 1px;
    height: 40px;
    background: var(--color-border);
    flex-shrink: 0;
  }
  .summary-stats {
    display: flex;
    gap: 28px;
  }
  .summary-stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .summary-val {
    font-family: var(--font-display);
    font-size: 20px;
    font-weight: 700;
    color: var(--color-text-primary);
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .summary-lbl {
    font-size: 10px;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  /* Sections */
  .section {
    margin-bottom: 32px;
  }
  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 12px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--color-border);
  }
  .section-head h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
    color: var(--color-text-primary);
    letter-spacing: -0.1px;
  }
  .section-count {
    font-size: 11px;
    color: var(--color-text-muted);
  }

  /* Cards */
  .card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    overflow: hidden;
  }

  /* Tables */
  .table-scroll { overflow-x: auto; }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  th {
    text-align: left;
    padding: 11px 16px;
    font-weight: 500;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--color-text-muted);
    background: var(--color-base);
    white-space: nowrap;
  }
  th.r { text-align: right; }
  td {
    padding: 11px 16px;
    color: var(--color-text-primary);
    border-top: 1px solid rgba(37, 45, 56, 0.4);
    vertical-align: middle;
  }
  tbody tr:first-child td { border-top: none; }
  tbody tr:hover td { background: rgba(255, 255, 255, 0.015); }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-secondary);
  }
  .risk {
    color: var(--color-yellow);
    font-weight: 600;
  }
  .cell-stack {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cell-name {
    font-weight: 500;
    color: var(--color-text-primary);
  }
  .cell-path {
    font-size: 10px;
    color: var(--color-text-muted);
  }
  .tag {
    font-size: 10px;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.04);
    color: var(--color-text-secondary);
  }

  /* Probability */
  .prob {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .prob-track {
    width: 80px;
    height: 4px;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 2px;
    overflow: hidden;
  }
  .prob-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.4s ease-out;
  }
  .prob-pct {
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--color-text-secondary);
    min-width: 28px;
    text-align: right;
  }

  /* Patterns */
  .pattern-row {
    padding: 14px 18px;
    border-top: 1px solid rgba(37, 45, 56, 0.4);
  }
  .pattern-row.first { border-top: none; }
  .pattern-row:hover { background: rgba(255, 255, 255, 0.015); }
  .pattern-main {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .pattern-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-primary);
  }
  .pattern-sev {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }
  .pattern-meta {
    font-size: 11px;
    color: var(--color-text-muted);
    display: flex;
    gap: 5px;
    margin-bottom: 4px;
  }
  .pattern-detail {
    font-size: 11px;
    color: var(--color-text-secondary);
  }

  /* API Surface */
  .api-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0;
  }
  .api-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 22px 16px;
    border-right: 1px solid var(--color-border);
  }
  .api-item:last-child { border-right: none; }
  .api-num {
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    color: var(--color-text-primary);
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }
  .api-label {
    font-size: 10px;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  /* Clean state */
  .clean-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 48px;
    color: var(--color-text-secondary);
    font-size: 13px;
  }
  .clean-icon {
    width: 28px;
    height: 28px;
    color: var(--color-green);
    opacity: 0.5;
  }

  @media (max-width: 768px) {
    .container { padding: 24px 20px 48px; }
    .summary { flex-direction: column; align-items: flex-start; gap: 16px; }
    .summary-divider { width: 100%; height: 1px; }
    .summary-stats { width: 100%; justify-content: space-between; }
    .api-grid { grid-template-columns: repeat(2, 1fr); }
    .api-item:nth-child(2) { border-right: none; }
    .api-item:nth-child(-n+2) { border-bottom: 1px solid var(--color-border); }
  }
</style>
