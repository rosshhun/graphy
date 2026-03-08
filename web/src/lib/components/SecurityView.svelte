<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type TaintPath, type ApiSurface } from '../api/client.js';
  import Badge from './Badge.svelte';
  import LoadingPulse from './LoadingPulse.svelte';

  let taintPaths = $state<TaintPath[]>([]);
  let apiSurface = $state<ApiSurface | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      const [t, a] = await Promise.all([api.taint(), api.apiSurface()]);
      taintPaths = t;
      apiSurface = a;
    } catch (e) {
      error = 'Failed to load security data. Is the server running?';
      console.error('Failed to load security data:', e);
    } finally {
      loading = false;
    }
  });

  let isClean = $derived(taintPaths.length === 0);
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
        <h1>Security</h1>
        <p>Taint analysis, attack surface, and data flow vulnerabilities</p>
      </header>

      <!-- Status Summary -->
      <div class="summary" class:clean={isClean} class:alert={!isClean}>
        <div class="status-row">
          <div class="status-icon" class:clean={isClean} class:alert={!isClean}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
              {#if isClean}
                <polyline points="9 12 11 14 15 10" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              {:else}
                <line x1="12" y1="9" x2="12" y2="13" stroke-width="2" stroke-linecap="round"/>
                <circle cx="12" cy="16" r="0.5" fill="currentColor" stroke="none"/>
              {/if}
            </svg>
          </div>
          <div class="status-text">
            <span class="status-title">
              {isClean ? 'No Vulnerabilities Detected' : `${taintPaths.length} Taint Path${taintPaths.length === 1 ? '' : 's'} Found`}
            </span>
            <span class="status-desc">
              {apiSurface?.public.length ?? 0} public API symbols &middot; {apiSurface ? apiSurface.public.length + taintPaths.length : 0} total attack surface
            </span>
          </div>
        </div>
        <div class="summary-stats">
          <div class="summary-stat">
            <span class="summary-val" style="color: {isClean ? 'var(--color-green)' : 'var(--color-red)'}">{taintPaths.length}</span>
            <span class="summary-lbl">Taint Paths</span>
          </div>
          <div class="summary-stat">
            <span class="summary-val">{apiSurface?.public.length ?? 0}</span>
            <span class="summary-lbl">Public API</span>
          </div>
          <div class="summary-stat">
            <span class="summary-val">{apiSurface ? apiSurface.public.length + taintPaths.length : 0}</span>
            <span class="summary-lbl">Attack Surface</span>
          </div>
        </div>
      </div>

      <!-- Taint Analysis -->
      <section class="section">
        <div class="section-head">
          <h2>Taint Analysis</h2>
          {#if taintPaths.length > 0}
            <span class="section-count">{taintPaths.length} paths</span>
          {/if}
        </div>

        {#if taintPaths.length === 0}
          <div class="card">
            <div class="clean-state">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="clean-icon">
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
                <polyline points="22 4 12 14.01 9 11.01"/>
              </svg>
              <span>No unsanitized data flows detected</span>
            </div>
          </div>
        {:else}
          <div class="card">
            {#each taintPaths as path, i}
              <div class="taint-row" class:first={i === 0}>
                <div class="taint-sink">
                  <span class="flow-tag critical">SINK</span>
                  <span class="flow-name">{path.target_name}</span>
                  <span class="flow-loc">{path.target_file}:{path.target_line}</span>
                </div>
                <div class="taint-sources">
                  {#each path.sources as source}
                    <div class="taint-source">
                      <span class="source-arrow">&larr;</span>
                      <span class="flow-tag source">SOURCE</span>
                      <Badge kind={source.kind} />
                      <span class="flow-name">{source.name}</span>
                      <span class="flow-loc">{source.file_path}:{source.start_line}</span>
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <!-- Public API Exposure -->
      {#if apiSurface && apiSurface.public.length > 0}
        <section class="section">
          <div class="section-head">
            <h2>Public API Exposure</h2>
            <span class="section-count">{apiSurface.public.length} symbols</span>
          </div>
          <div class="card">
            <div class="table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>Symbol</th>
                    <th>Kind</th>
                    <th>Signature</th>
                    <th class="r">External Callers</th>
                  </tr>
                </thead>
                <tbody>
                  {#each apiSurface.public.slice(0, 20) as sym}
                    <tr>
                      <td>
                        <div class="cell-stack">
                          <span class="cell-name">{sym.name}</span>
                          <span class="cell-path">{sym.file_path}:{sym.start_line}</span>
                        </div>
                      </td>
                      <td><Badge kind={sym.kind} /></td>
                      <td class="sig">{sym.signature || '-'}</td>
                      <td class="num">{sym.external_callers}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
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
    justify-content: space-between;
    gap: 24px;
    padding: 22px 26px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    margin-bottom: 36px;
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 16px;
    flex: 1;
    min-width: 0;
  }
  .status-icon {
    width: 40px;
    height: 40px;
    border-radius: 3px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .status-icon svg {
    width: 22px;
    height: 22px;
  }
  .status-icon.clean {
    background: rgba(74, 222, 128, 0.08);
    color: var(--color-green);
  }
  .status-icon.alert {
    background: rgba(248, 113, 113, 0.08);
    color: var(--color-red);
  }
  .status-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .status-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text-primary);
  }
  .status-desc {
    font-size: 11px;
    color: var(--color-text-muted);
  }
  .summary-stats {
    display: flex;
    gap: 24px;
    flex-shrink: 0;
  }
  .summary-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
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

  /* Taint rows */
  .taint-row {
    padding: 16px 20px;
    border-top: 1px solid rgba(37, 45, 56, 0.4);
  }
  .taint-row.first { border-top: none; }
  .taint-row:hover { background: rgba(255, 255, 255, 0.015); }
  .taint-sink {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .flow-tag {
    font-size: 9px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 3px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    flex-shrink: 0;
  }
  .flow-tag.critical {
    background: rgba(248, 113, 113, 0.1);
    color: var(--color-red);
  }
  .flow-tag.source {
    background: rgba(251, 191, 36, 0.1);
    color: var(--color-yellow);
  }
  .flow-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--color-text-primary);
  }
  .flow-loc {
    font-size: 10px;
    color: var(--color-text-muted);
  }
  .taint-sources {
    padding-left: 14px;
    border-left: 1px solid rgba(248, 113, 113, 0.15);
    margin-left: 4px;
  }
  .taint-source {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    flex-wrap: wrap;
  }
  .source-arrow {
    font-size: 12px;
    color: var(--color-text-muted);
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
  .sig {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--color-text-secondary);
  }
  .cell-stack { display: flex; flex-direction: column; gap: 2px; }
  .cell-name { font-weight: 500; color: var(--color-text-primary); }
  .cell-path { font-size: 10px; color: var(--color-text-muted); }

  @media (max-width: 768px) {
    .container { padding: 24px 20px 48px; }
    .summary { flex-direction: column; align-items: flex-start; }
    .summary-stats { width: 100%; justify-content: space-between; }
  }
</style>
