<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type GraphData, type SymbolDetail as SymbolDetailType, type FileContent, type FileSymbolsResponse, type FileSymbol } from '../api/client.js';
  import { app } from '../stores/app.svelte.js';
  import { fileIcon } from '../icons.js';
  import FileTree from './FileTree.svelte';
  import GraphCanvas from './GraphCanvas.svelte';
  import SymbolDetail from './SymbolDetail.svelte';
  import CodeViewer from './CodeViewer.svelte';
  import SymbolsPanel from './SymbolsPanel.svelte';
  import ReferencesPanel from './ReferencesPanel.svelte';
  import LoadingPulse from './LoadingPulse.svelte';

  let files = $state<string[]>([]);
  let graphData = $state<GraphData | null>(null);
  let selectedSymbol = $state<SymbolDetailType | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Code view state — shared via store so bottom bar can toggle it
  let mode = $derived(app.explorerMode);
  let fileContent = $state<FileContent | null>(null);
  let fileSymbols = $state<FileSymbolsResponse | null>(null);
  let selectedFilePath = $state<string | null>(null);
  let highlightLines = $state<number[]>([]);
  let fileLoading = $state(false);

  // Graph focus state
  let focusedFile = $state<string | null>(null);

  let focusedFileStats = $derived.by(() => {
    if (!focusedFile || !graphData) return null;
    const nodeIds = new Set<string>();
    for (const n of graphData.nodes) {
      if (n.file === focusedFile) nodeIds.add(n.id);
    }
    let edgeCount = 0;
    for (const e of graphData.edges) {
      if (nodeIds.has(e.source) || nodeIds.has(e.target)) edgeCount++;
    }
    return { nodeCount: nodeIds.size, edgeCount };
  });

  onMount(async () => {
    try {
      const [f, g] = await Promise.all([api.files(), api.graph()]);
      files = f;
      graphData = g;
    } catch (e) {
      error = 'Failed to load explorer data. Is the server running?';
      console.error('Failed to load explorer data:', e);
    } finally {
      loading = false;
    }
  });

  $effect(() => {
    const name = app.pendingSymbolName;
    if (name) {
      app.pendingSymbolName = null;
      selectSymbol(name);
    }
  });

  async function selectSymbol(name: string) {
    try {
      const details = await api.symbol(name);
      if (details.length > 0) {
        selectedSymbol = details[0];
        app.selectedSymbol = details[0];
        if (mode === 'code' && details[0].file_path !== selectedFilePath) {
          await loadFile(details[0].file_path);
        }
        if (mode === 'code') {
          const lines: number[] = [];
          for (let i = details[0].start_line; i <= details[0].end_line; i++) {
            lines.push(i);
          }
          highlightLines = lines;
        }
      }
    } catch (e) {
      console.error('Failed to load symbol:', e);
    }
  }

  async function loadFile(path: string) {
    if (path === selectedFilePath && fileContent) return;
    fileLoading = true;
    selectedFilePath = path;
    try {
      const [fc, fs] = await Promise.all([
        api.fileContent(path),
        api.fileSymbols(path),
      ]);
      fileContent = fc;
      fileSymbols = fs;
    } catch (e) {
      console.error('Failed to load file:', e);
      fileContent = null;
      fileSymbols = null;
    } finally {
      fileLoading = false;
    }
  }

  function handleFileSelect(file: string) {
    if (mode === 'graph') {
      // Focus file in graph instead of switching to code mode
      focusedFile = file;
      selectedSymbol = null;
      app.selectedSymbol = null;
    } else {
      selectedSymbol = null;
      app.selectedSymbol = null;
      highlightLines = [];
      loadFile(file);
    }
  }

  function handleClearFocus() {
    focusedFile = null;
  }

  function openFocusedInCode() {
    if (focusedFile) {
      app.explorerMode = 'code';
      loadFile(focusedFile);
      focusedFile = null;
    }
  }

  function handleSymbolClick(sym: FileSymbol) {
    const lines: number[] = [];
    for (let i = sym.start_line; i <= sym.end_line; i++) {
      lines.push(i);
    }
    highlightLines = lines;
    selectSymbol(sym.name);
  }

  function handleLineClick(line: number) {
    if (!fileSymbols) return;
    for (const sym of fileSymbols.symbols) {
      for (const child of sym.children) {
        if (line >= child.start_line && line <= child.end_line) {
          handleSymbolClick(child);
          return;
        }
      }
      if (line >= sym.start_line && line <= sym.end_line) {
        handleSymbolClick(sym);
        return;
      }
    }
  }

  function switchToGraph() {
    app.explorerMode = 'graph';
    highlightLines = [];
  }

  function switchToCode() {
    app.explorerMode = 'code';
    if (focusedFile) {
      loadFile(focusedFile);
      focusedFile = null;
    } else if (!selectedFilePath && files.length > 0) {
      loadFile(files[0]);
    }
  }

  let hasSymbols = $derived(fileSymbols !== null && fileSymbols.symbol_count > 0);
</script>

<div
  class="explorer"
  class:has-detail={mode === 'graph' && selectedSymbol !== null}
  class:code-mode={mode === 'code'}
  class:has-symbols={mode === 'code' && hasSymbols}
>
  {#if loading}
    <div class="explorer-loading"><LoadingPulse /></div>
  {:else if error}
    <div class="explorer-error"><p>{error}</p></div>
  {:else}
    <aside class="sidebar" class:open={app.sidebarOpen}>
      {#if app.sidebarOpen}
        <div class="sidebar-header">
          <span class="sidebar-title">Files</span>
          <button class="toggle-btn" onclick={() => app.sidebarOpen = false} title="Collapse sidebar">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10 3L5 8l5 5"/>
            </svg>
          </button>
        </div>
        <FileTree {files} onSelect={handleFileSelect} />
      {:else}
        <button class="collapsed-toggle" onclick={() => app.sidebarOpen = true} title="Expand sidebar">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M6 3l5 5-5 5"/>
          </svg>
          <span class="collapsed-label">Files</span>
        </button>
      {/if}
    </aside>

    {#if mode === 'graph'}
      <!-- ── Graph Mode ── -->
      <main class="graph-area">
        <GraphCanvas data={graphData} onNodeClick={selectSymbol} {focusedFile} onFocusCleared={handleClearFocus} />

        {#if focusedFile && focusedFileStats}
          {@const parts = focusedFile.split('/')}
          {@const showParts = parts.length > 4 ? ['…', ...parts.slice(-3)] : parts}
          <div class="focus-indicator">
            <span class="fi-icon">{@html fileIcon(parts[parts.length - 1] || '', 14)}</span>
            <span class="fi-path">
              {#each showParts as part, i}
                {#if i > 0}<span class="fi-sep">/</span>{/if}
                <span class="fi-part" class:fi-last={i === showParts.length - 1}>{part}</span>
              {/each}
            </span>
            <span class="fi-divider"></span>
            <span class="fi-stat">{focusedFileStats.nodeCount} symbols</span>
            <span class="fi-dot"></span>
            <span class="fi-stat">{focusedFileStats.edgeCount} connections</span>
            <button class="fi-code-btn" onclick={openFocusedInCode} title="View code">
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="5.5 3 1.5 8 5.5 13"/><polyline points="10.5 3 14.5 8 10.5 13"/>
              </svg>
              Code
            </button>
            <button class="fi-close" onclick={handleClearFocus} title="Clear focus">
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
                <line x1="4" y1="4" x2="12" y2="12"/><line x1="12" y1="4" x2="4" y2="12"/>
              </svg>
            </button>
          </div>
        {/if}

      </main>

      {#if selectedSymbol}
        <aside class="detail-sidebar">
          <SymbolDetail
            symbol={selectedSymbol}
            onClose={() => { selectedSymbol = null; app.selectedSymbol = null; }}
            onNavigate={selectSymbol}
          />
        </aside>
      {/if}

    {:else}
      <!-- ── Code Mode ── -->
      <main class="code-area">
        {#if fileLoading}
          <div class="code-loading"><LoadingPulse /></div>
        {:else if fileContent}
          <div class="code-main">
            <CodeViewer
              file={fileContent}
              {highlightLines}
              onLineClick={handleLineClick}
            />
            {#if selectedSymbol}
              <ReferencesPanel
                symbol={selectedSymbol}
                onNavigate={selectSymbol}
              />
            {/if}
          </div>
        {:else}
          <div class="code-empty">
            <svg class="ce-icon" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="8" y="6" width="32" height="36" rx="3"/>
              <line x1="16" y1="16" x2="32" y2="16" opacity="0.3"/>
              <line x1="16" y1="22" x2="28" y2="22" opacity="0.3"/>
              <line x1="16" y1="28" x2="30" y2="28" opacity="0.3"/>
            </svg>
            <p>Select a file to view its contents</p>
          </div>
        {/if}
      </main>

      {#if hasSymbols}
        <aside class="symbols-sidebar">
          <SymbolsPanel
            data={fileSymbols!}
            onSymbolClick={handleSymbolClick}
            activeSymbol={selectedSymbol?.name ?? null}
          />
        </aside>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .explorer {
    display: grid;
    grid-template-columns: auto 1fr;
    height: 100%;
    overflow: hidden;
  }
  .explorer.has-detail {
    grid-template-columns: auto 1fr 320px;
  }
  .explorer.code-mode {
    grid-template-columns: auto 1fr;
  }
  .explorer.code-mode.has-symbols {
    grid-template-columns: auto 1fr 280px;
  }
  .explorer-loading,
  .explorer-error {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .explorer-error {
    color: var(--color-text-secondary, #858585);
    font-size: 13px;
  }

  /* ── File sidebar ── */
  .sidebar {
    background: var(--color-bg, #0a0a0a);
    border-right: 1px solid var(--color-border, #1e1e1e);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    width: 240px;
  }
  .sidebar:not(.open) {
    width: 40px;
  }
  .sidebar-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    border-bottom: 1px solid var(--color-border, #1e1e1e);
    flex-shrink: 0;
  }
  .sidebar-title {
    font-family: var(--font-display);
    font-size: 13px;
    color: var(--color-text-secondary, #858585);
    font-weight: 600;
    letter-spacing: 0.2px;
  }
  .toggle-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: 6px;
    color: var(--color-text-muted, #4a4a4a);
    cursor: pointer;
    padding: 0;
    transition: color 0.12s, background 0.12s;
  }
  .toggle-btn:hover {
    color: var(--color-text-secondary, #858585);
    background: var(--color-surface-hover, #1a1a1a);
  }
  .collapsed-toggle {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 12px 0;
    background: none;
    border: none;
    border-bottom: 1px solid var(--color-border, #1e1e1e);
    color: var(--color-text-muted, #4a4a4a);
    cursor: pointer;
    transition: color 0.12s, background 0.12s;
  }
  .collapsed-toggle:hover {
    color: var(--color-text-secondary, #858585);
    background: var(--color-surface-hover, #1a1a1a);
  }
  .collapsed-label {
    writing-mode: vertical-rl;
    text-orientation: mixed;
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.5px;
  }

  /* ── Graph mode ── */
  .graph-area {
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* ── Code mode ── */
  .code-area {
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .code-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .code-loading {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .code-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--color-text-muted);
    font-family: var(--font-display);
    font-size: 14px;
  }
  .ce-icon {
    width: 48px;
    height: 48px;
    opacity: 0.4;
  }

  /* ── Sidebars ── */
  .detail-sidebar {
    background: transparent;
    border-left: 1px solid var(--color-border, #1e1e1e);
    overflow: hidden;
  }
  .symbols-sidebar {
    background: transparent;
    border-left: 1px solid var(--color-border, #1e1e1e);
    overflow: hidden;
  }


  /* ── Focus indicator ── */
  .focus-indicator {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 15;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 10px;
    background: rgba(18, 18, 18, 0.88);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: nowrap;
    animation: fi-slide-in 0.2s ease-out;
  }

  @keyframes fi-slide-in {
    from { opacity: 0; transform: translateX(-50%) translateY(-8px); }
    to { opacity: 1; transform: translateX(-50%) translateY(0); }
  }

  .fi-icon {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .fi-path {
    display: flex;
    align-items: center;
    gap: 2px;
    color: var(--color-text-secondary);
  }

  .fi-sep {
    color: var(--color-text-muted);
  }

  .fi-part {
    color: var(--color-text-secondary);
  }

  .fi-last {
    color: var(--color-text-primary);
    font-weight: 500;
  }

  .fi-divider {
    width: 1px;
    height: 14px;
    background: var(--color-border);
    flex-shrink: 0;
  }

  .fi-stat {
    color: var(--color-text-muted);
    font-size: 11px;
  }

  .fi-dot {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--color-text-muted);
    flex-shrink: 0;
  }

  .fi-code-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    background: rgba(7, 119, 255, 0.12);
    border: 1px solid rgba(7, 119, 255, 0.25);
    border-radius: 4px;
    color: var(--color-accent);
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.12s;
  }

  .fi-code-btn:hover {
    background: rgba(7, 119, 255, 0.2);
  }

  .fi-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    background: none;
    border: none;
    border-radius: 4px;
    color: var(--color-text-muted);
    cursor: pointer;
    padding: 0;
    transition: color 0.12s, background 0.12s;
  }

  .fi-close:hover {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  @media (max-width: 1024px) {
    .explorer { grid-template-columns: 1fr; }
    .sidebar { display: none; }
    .explorer.has-detail { grid-template-columns: 1fr; }
    .explorer.code-mode { grid-template-columns: 1fr; }
    .explorer.code-mode.has-symbols { grid-template-columns: 1fr; }
    .detail-sidebar,
    .symbols-sidebar {
      position: absolute;
      right: 0;
      top: 0;
      bottom: 0;
      width: 300px;
      z-index: 20;
      box-shadow: -4px 0 24px rgba(0,0,0,0.6);
      background: var(--color-bg);
    }
  }
</style>
