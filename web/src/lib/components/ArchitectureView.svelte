<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ArchitectureData, type Stats } from '../api/client.js';
  import BarChart from './BarChart.svelte';
  import LoadingPulse from './LoadingPulse.svelte';

  let arch = $state<ArchitectureData | null>(null);
  let stats = $state<Stats | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      const [a, s] = await Promise.all([api.architecture(), api.stats()]);
      arch = a;
      stats = s;
    } catch (e) {
      error = 'Failed to load architecture data. Is the server running?';
      console.error('Failed to load architecture data:', e);
    } finally {
      loading = false;
    }
  });

  let langItems = $derived(
    arch?.languages.map(l => ({ label: l.language, value: l.count })) ?? []
  );
  let kindItems = $derived(
    arch?.kind_distribution.map(k => ({ label: k.kind, value: k.count })) ?? []
  );
  let edgeItems = $derived(
    arch?.edge_distribution.map(e => ({ label: e.kind, value: e.count })) ?? []
  );

  const primaryStats = [
    { key: 'nodes', label: 'Nodes' },
    { key: 'edges', label: 'Edges' },
    { key: 'files', label: 'Files' },
  ] as const;

  const detailStats = [
    { key: 'functions', label: 'Functions' },
    { key: 'classes', label: 'Classes' },
    { key: 'methods', label: 'Methods' },
    { key: 'structs', label: 'Structs' },
    { key: 'traits', label: 'Traits' },
    { key: 'enums', label: 'Enums' },
    { key: 'imports', label: 'Imports' },
  ] as const;
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
        <h1>Architecture</h1>
        <p>Graph structure, language breakdown, and codebase composition</p>
      </header>

      <!-- Overview Numbers -->
      {#if stats}
        <div class="overview">
          <div class="overview-primary">
            {#each primaryStats as def}
              <div class="ov-big">
                <span class="ov-big-val">{(stats[def.key] as number).toLocaleString()}</span>
                <span class="ov-big-lbl">{def.label}</span>
              </div>
            {/each}
          </div>
          <div class="overview-detail">
            {#each detailStats as def}
              <div class="ov-item">
                <span class="ov-item-val">{(stats[def.key] as number).toLocaleString()}</span>
                <span class="ov-item-lbl">{def.label}</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Charts Grid -->
      {#if arch}
        <div class="charts-grid">
          {#if langItems.length > 0}
            <section class="section">
              <div class="section-head">
                <h2>Language Distribution</h2>
                <span class="section-count">{langItems.length} languages</span>
              </div>
              <div class="card chart-card">
                <BarChart items={langItems} />
              </div>
            </section>
          {/if}

          {#if kindItems.length > 0}
            <section class="section">
              <div class="section-head">
                <h2>Node Kinds</h2>
                <span class="section-count">{kindItems.length} kinds</span>
              </div>
              <div class="card chart-card">
                <BarChart items={kindItems} colorByKind />
              </div>
            </section>
          {/if}

          {#if edgeItems.length > 0}
            <section class="section">
              <div class="section-head">
                <h2>Edge Kinds</h2>
                <span class="section-count">{edgeItems.length} kinds</span>
              </div>
              <div class="card chart-card">
                <BarChart items={edgeItems} />
              </div>
            </section>
          {/if}

          {#if arch.largest_files.length > 0}
            <section class="section">
              <div class="section-head">
                <h2>Largest Files</h2>
                <span class="section-count">{arch.largest_files.length} files</span>
              </div>
              <div class="card">
                <div class="table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>File</th>
                        <th class="r">Symbols</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each arch.largest_files as file}
                        <tr>
                          <td>
                            <span class="filepath">{file.path}</span>
                          </td>
                          <td class="num">
                            <div class="file-num">
                              <div class="file-bar">
                                <div
                                  class="file-bar-fill"
                                  style="width: {(file.symbol_count / Math.max(...arch.largest_files.map(f => f.symbol_count), 1)) * 100}%"
                                ></div>
                              </div>
                              <span>{file.symbol_count}</span>
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
        </div>
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

  /* Overview */
  .overview {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    margin-bottom: 36px;
    overflow: hidden;
  }
  .overview-primary {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    border-bottom: 1px solid var(--color-border);
  }
  .ov-big {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 24px 16px;
    border-right: 1px solid var(--color-border);
  }
  .ov-big:last-child { border-right: none; }
  .ov-big-val {
    font-family: var(--font-display);
    font-size: 28px;
    font-weight: 700;
    color: var(--color-text-primary);
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .ov-big-lbl {
    font-size: 10px;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .overview-detail {
    display: flex;
    justify-content: center;
    gap: 0;
    padding: 0;
  }
  .ov-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 14px 20px;
    border-right: 1px solid var(--color-border);
    flex: 1;
  }
  .ov-item:last-child { border-right: none; }
  .ov-item-val {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 600;
    color: var(--color-text-primary);
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }
  .ov-item-lbl {
    font-size: 9px;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  /* Charts Grid */
  .charts-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 24px 20px;
  }

  /* Sections */
  .section {
    min-width: 0;
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
  .chart-card {
    padding: 18px 20px;
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
  }
  .filepath {
    font-size: 11px;
    color: var(--color-text-secondary);
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-num {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: flex-end;
  }
  .file-bar {
    width: 60px;
    height: 4px;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 2px;
    overflow: hidden;
    flex-shrink: 0;
  }
  .file-bar-fill {
    height: 100%;
    background: var(--color-accent);
    border-radius: 2px;
    transition: width 0.5s ease-out;
  }

  @media (max-width: 1024px) {
    .charts-grid { grid-template-columns: 1fr; }
  }
  @media (max-width: 768px) {
    .container { padding: 24px 20px 48px; }
    .overview-primary { grid-template-columns: 1fr 1fr 1fr; }
    .overview-detail { flex-wrap: wrap; }
    .ov-item { min-width: 0; padding: 12px 14px; }
  }
</style>
