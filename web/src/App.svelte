<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './lib/api/client.js';
  import { app, KIND_COLORS } from './lib/stores/app.svelte.js';

  const allLegendItems = [
    { label: 'Function', key: 'Function', stat: 'functions' },
    { label: 'Class', key: 'Class', stat: 'classes' },
    { label: 'Method', key: 'Method', stat: 'methods' },
    { label: 'Struct', key: 'Struct', stat: 'structs' },
    { label: 'Trait', key: 'Trait', stat: 'traits' },
    { label: 'Enum', key: 'Enum', stat: 'enums' },
    { label: 'File', key: 'File', stat: 'files' },
  ] as const;

  let legendItems = $derived(
    app.stats
      ? allLegendItems.filter(item => (app.stats as any)[item.stat] > 0)
      : allLegendItems
  );
  import Nav from './lib/components/Nav.svelte';
  import ExplorerView from './lib/components/ExplorerView.svelte';
  import SearchView from './lib/components/SearchView.svelte';
  import AnalysisView from './lib/components/AnalysisView.svelte';
  import SecurityView from './lib/components/SecurityView.svelte';
  import ArchitectureView from './lib/components/ArchitectureView.svelte';

  onMount(async () => {
    try {
      app.stats = await api.stats();
    } catch (e) {
      console.error('Failed to load stats:', e);
    }
  });

  // Track which views have been visited so we keep them mounted once loaded
  let visitedExplorer = $state(true);
  let visitedSearch = $state(false);
  let visitedAnalysis = $state(false);
  let visitedSecurity = $state(false);
  let visitedArchitecture = $state(false);

  $effect(() => {
    const v = app.activeView;
    if (v === 'analysis') visitedAnalysis = true;
    if (v === 'security') visitedSecurity = true;
    if (v === 'architecture') visitedArchitecture = true;
  });

  $effect(() => {
    if (app.searchActive) visitedSearch = true;
  });

  // Search bar state
  let searchQuery = $state('');
  let searchInput = $state<HTMLInputElement | undefined>(undefined);

  function handleSearchSubmit() {
    if (searchQuery.length >= 1) {
      app.searchActive = true;
      app.pendingSearchQuery = searchQuery;
      visitedSearch = true;
    }
  }

  function handleSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (searchQuery) {
        searchQuery = '';
      } else {
        app.searchActive = false;
        searchInput?.blur();
      }
    } else if (e.key === 'Enter') {
      handleSearchSubmit();
    }
  }

  function handleSearchFocus() {
    app.searchActive = true;
    visitedSearch = true;
  }

  $effect(() => {
    function handleGlobalKeydown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        app.searchActive = true;
        visitedSearch = true;
        setTimeout(() => searchInput?.focus(), 50);
      }
    }
    window.addEventListener('keydown', handleGlobalKeydown);
    return () => window.removeEventListener('keydown', handleGlobalKeydown);
  });
</script>

<div class="app-layout">
  <Nav />

  <div class="main-area">
    <!-- Always-visible search bar -->
    <div class="top-search-bar">
      <div class="search-box" class:focused={app.searchActive}>
        <svg class="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        <input
          bind:this={searchInput}
          type="text"
          placeholder="Search symbols, functions, classes..."
          bind:value={searchQuery}
          oninput={handleSearchSubmit}
          onkeydown={handleSearchKeydown}
          onfocus={handleSearchFocus}
        />
        <span class="search-filter-hint">file: kind: lang:</span>
        {#if app.searchActive && searchQuery}
          <button class="search-clear" title="Clear search" onclick={() => { searchQuery = ''; app.searchActive = false; }}>
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M4 4l8 8M12 4l-8 8"/>
            </svg>
          </button>
        {/if}
      </div>
    </div>

    <!-- Content area -->
    <div class="view-container">
      <!-- Search results overlay -->
      <div class="view-panel" class:active={app.searchActive}>
        {#if visitedSearch}
          <SearchView bind:query={searchQuery} />
        {/if}
      </div>

      <!-- Regular views underneath -->
      <div class="view-panel" class:active={app.activeView === 'explorer' && !app.searchActive}>
        {#if visitedExplorer}
          <ExplorerView />
        {/if}
      </div>

      <div class="view-panel" class:active={app.activeView === 'analysis' && !app.searchActive}>
        {#if visitedAnalysis}
          <AnalysisView />
        {/if}
      </div>

      <div class="view-panel" class:active={app.activeView === 'security' && !app.searchActive}>
        {#if visitedSecurity}
          <SecurityView />
        {/if}
      </div>

      <div class="view-panel" class:active={app.activeView === 'architecture' && !app.searchActive}>
        {#if visitedArchitecture}
          <ArchitectureView />
        {/if}
      </div>
    </div>

    {#if app.activeView === 'explorer' && !app.searchActive}
      <div class="bottom-bar">
        <div class="bottom-legend">
          {#each legendItems as item}
            <div class="legend-item">
              <div class="legend-dot" style="background: {KIND_COLORS[item.key] || '#6b7280'}"></div>
              {item.label}
            </div>
          {/each}
        </div>
        <div class="bottom-zoom">
          <button class="zoom-btn" title="Zoom in" disabled={!app.zoomIn} onclick={() => app.zoomIn?.()}>+</button>
          <button class="zoom-btn" title="Zoom out" disabled={!app.zoomOut} onclick={() => app.zoomOut?.()}>&minus;</button>
          <button class="zoom-btn" title="Reset zoom" disabled={!app.zoomReset} onclick={() => app.zoomReset?.()}>&#x27F2;</button>
        </div>
        {#if app.stats}
          <div class="bottom-stats">
            <span class="stat-pill">{app.stats.files} files</span>
            <span class="stat-pill">{app.stats.nodes} symbols</span>
            <span class="stat-pill">{app.stats.edges} edges</span>
          </div>
        {/if}
        <div class="bottom-mode-switch">
          <button
            class="bottom-mode-btn"
            class:active={app.explorerMode === 'graph'}
            onclick={() => app.explorerMode = 'graph'}
            title="Graph view"
          >
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
              <circle cx="4" cy="4" r="2"/><circle cx="12" cy="4" r="2"/><circle cx="8" cy="13" r="2"/>
              <line x1="6" y1="4.5" x2="10" y2="4.5"/><line x1="5" y1="5.5" x2="7" y2="11"/><line x1="11" y1="5.5" x2="9" y2="11"/>
            </svg>
            Graph
          </button>
          <button
            class="bottom-mode-btn"
            class:active={app.explorerMode === 'code'}
            onclick={() => app.explorerMode = 'code'}
            title="Code view"
          >
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="5.5 3 1.5 8 5.5 13"/><polyline points="10.5 3 14.5 8 10.5 13"/>
            </svg>
            Code
          </button>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .app-layout {
    display: flex;
    flex-direction: row;
    height: 100vh;
    overflow: hidden;
    background: var(--color-bg);
    gap: 0;
  }

  .main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
    background: var(--color-bg);
    border-radius: 12px;
    margin: 6px 6px 6px 0;
    border: 1px solid var(--color-border);
  }

  /* ── Top search bar ── */
  .top-search-bar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 18px;
    background: transparent;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    height: 50px;
  }

  .search-box {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 12px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 7px;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .search-box:hover {
    border-color: var(--color-border-bright);
  }

  .search-box.focused {
    border-color: var(--sg-theme-border-color-focus);
    box-shadow: var(--sg-theme-shadow-focus);
  }

  .search-icon {
    width: 16px;
    height: 16px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    transition: color 0.15s;
  }

  .search-box.focused .search-icon {
    color: var(--color-accent);
  }

  .search-box input {
    flex: 1;
    background: none;
    border: none;
    outline: none;
    color: var(--color-text-primary);
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 400;
    min-width: 0;
  }

  .search-box input::placeholder {
    color: var(--color-text-muted);
    font-weight: 400;
  }

  .search-filter-hint {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    opacity: 0.5;
    letter-spacing: 0.5px;
    pointer-events: none;
    user-select: none;
  }

  .search-clear {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text-muted);
    padding: 0;
    flex-shrink: 0;
    border-radius: 3px;
    transition: color 0.1s, background 0.1s;
  }

  .search-clear:hover {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  .search-clear svg {
    width: 12px;
    height: 12px;
  }


  .bottom-bar {
    display: flex;
    align-items: center;
    padding: 4px 18px;
    border-top: 1px solid var(--color-border);
    flex-shrink: 0;
    font-size: 11px;
    gap: 14px;
  }

  .bottom-mode-switch {
    display: flex;
    align-items: center;
    gap: 1px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    padding: 1px;
    flex-shrink: 0;
  }

  .bottom-mode-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: 11px;
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
    line-height: 1.4;
  }

  .bottom-mode-btn:hover {
    color: var(--color-text-secondary);
  }

  .bottom-mode-btn.active {
    background: var(--color-elevated);
    color: var(--color-text-primary);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .bottom-mode-btn svg {
    flex-shrink: 0;
  }

  .bottom-legend {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: 1;
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: 11px;
  }

  .legend-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .bottom-zoom {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .zoom-btn {
    width: 24px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: 4px;
    color: var(--color-text-muted);
    cursor: pointer;
    font-size: 13px;
    font-family: var(--font-mono);
    padding: 0;
    transition: color 0.12s, background 0.12s;
  }

  .zoom-btn:hover:not(:disabled) {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  .zoom-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .bottom-stats {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .stat-pill {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    padding: 1px 6px;
  }

  /* ── Views ── */
  .view-container {
    flex: 1;
    position: relative;
    overflow: hidden;
  }

  .view-panel {
    position: absolute;
    inset: 0;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.15s ease;
  }

  .view-panel.active {
    opacity: 1;
    pointer-events: auto;
  }

  @media (max-width: 768px) {
    .bottom-bar { display: none; }
  }
</style>
