<script lang="ts">
  import type { SearchResult, FileContent } from '../api/client.js';
  import { api } from '../api/client.js';
  import { app, KIND_COLORS, KIND_ABBREV, LANG_COLORS } from '../stores/app.svelte.js';
  import { langIcon, kindIcon, fileIcon } from '../icons.js';
  import CodeViewer from './CodeViewer.svelte';

  let { query = $bindable('') }: { query: string } = $props();

  let results = $state<SearchResult[]>([]);
  let isSearching = $state(false);
  let hasSearched = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout>;
  let searchTime = $state<number | null>(null);

  let activeKinds = $state(new Set<string>());
  let activeLangs = $state(new Set<string>());
  let activeFiles = $state(new Set<string>());

  // Preview state
  let previewFile = $state<FileContent | null>(null);
  let previewFilePath = $state<string | null>(null);
  let previewMatchLines = $state<number[]>([]);
  let previewMatchIndex = $state(0);
  let previewLoading = $state(false);

  // Handle query passed from App's search bar
  $effect(() => {
    if (app.pendingSearchQuery && app.searchActive) {
      const q = app.pendingSearchQuery;
      app.pendingSearchQuery = null;
      query = q;
      executeSearch();
    }
  });

  // React to query changes from parent
  $effect(() => {
    const { text, kind, lang, file } = parseFilters(query);
    const hasFilters = !!(kind || lang || file);
    if (text.length >= 2 || hasFilters) {
      clearTimeout(debounceTimer);
      activeKinds = new Set();
      activeLangs = new Set();
      activeFiles = new Set();
      hasSearched = true;
      isSearching = true;
      debounceTimer = setTimeout(executeSearch, 250);
    } else if (query.length === 0) {
      results = [];
      hasSearched = false;
      searchTime = null;
    }
  });

  /**
   * Parse filter prefixes from the query string.
   * Supports: file:pattern, kind:Function, lang:Rust, -file:test
   * Remaining text after extracting filters becomes the search query.
   */
  function parseFilters(raw: string): { text: string; kind?: string; lang?: string; file?: string } {
    let kind: string | undefined;
    let lang: string | undefined;
    let file: string | undefined;
    const textParts: string[] = [];

    // Split on spaces but preserve quoted strings
    const tokens = raw.match(/(?:[^\s"]+|"[^"]*")+/g) || [];
    for (const token of tokens) {
      const lower = token.toLowerCase();
      if (lower.startsWith('kind:')) {
        kind = token.slice(5).replace(/^"|"$/g, '');
      } else if (lower.startsWith('lang:')) {
        lang = token.slice(5).replace(/^"|"$/g, '');
      } else if (lower.startsWith('file:')) {
        file = token.slice(5).replace(/^"|"$/g, '');
      } else {
        textParts.push(token);
      }
    }
    return { text: textParts.join(' '), kind, lang, file };
  }

  // Parsed filters derived from query
  let parsed = $derived(parseFilters(query));

  async function executeSearch() {
    const { text, kind, lang, file } = parseFilters(query);
    if (text.length < 2 && !kind && !lang && !file) return;
    isSearching = true;
    hasSearched = true;
    const start = performance.now();
    try {
      const opts: { kind?: string; lang?: string; file?: string } = {};
      if (kind) opts.kind = kind;
      if (lang) opts.lang = lang;
      if (file) opts.file = file;
      results = await api.search(text, 200, Object.keys(opts).length > 0 ? opts : undefined);
      searchTime = performance.now() - start;
    } catch {
      results = [];
      searchTime = null;
    }
    finally { isSearching = false; }
  }

  // ── Preview ──
  async function openPreview(filePath: string, matchLines: number[]) {
    previewFilePath = filePath;
    previewMatchLines = matchLines;
    previewMatchIndex = 0;
    previewLoading = true;
    try {
      previewFile = await api.fileContent(filePath);
    } catch {
      previewFile = null;
    } finally {
      previewLoading = false;
    }
  }

  function closePreview() {
    previewFile = null;
    previewFilePath = null;
    previewMatchLines = [];
  }

  function prevMatch() {
    if (previewMatchIndex > 0) {
      previewMatchIndex--;
    }
  }

  function nextMatch() {
    if (previewMatchIndex < previewMatchLines.length - 1) {
      previewMatchIndex++;
    }
  }

  let currentHighlight = $derived(
    previewMatchLines.length > 0 ? [previewMatchLines[previewMatchIndex]] : []
  );

  // ── Facets ──
  let kindFacets = $derived.by(() => {
    const m = new Map<string, number>();
    for (const r of results) m.set(r.kind, (m.get(r.kind) || 0) + 1);
    return [...m].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count);
  });

  let langFacets = $derived.by(() => {
    const m = new Map<string, number>();
    for (const r of results) if (r.language) m.set(r.language, (m.get(r.language) || 0) + 1);
    return [...m].map(([lang, count]) => ({ lang, count })).sort((a, b) => b.count - a.count);
  });

  let fileFacets = $derived.by(() => {
    const m = new Map<string, number>();
    for (const r of results) {
      const dir = r.file_path.includes('/')
        ? r.file_path.substring(0, r.file_path.lastIndexOf('/'))
        : r.file_path;
      m.set(dir, (m.get(dir) || 0) + 1);
    }
    return [...m].map(([file, count]) => ({ file, count })).sort((a, b) => b.count - a.count).slice(0, 10);
  });

  // ── Filtered + Grouped ──
  let filteredResults = $derived.by(() => {
    return results.filter(r => {
      if (activeKinds.size > 0 && !activeKinds.has(r.kind)) return false;
      if (activeLangs.size > 0 && !activeLangs.has(r.language)) return false;
      if (activeFiles.size > 0) {
        const dir = r.file_path.includes('/')
          ? r.file_path.substring(0, r.file_path.lastIndexOf('/'))
          : r.file_path;
        if (!activeFiles.has(dir)) return false;
      }
      return true;
    });
  });

  let groupedResults = $derived.by(() => {
    const groups = new Map<string, SearchResult[]>();
    for (const r of filteredResults) {
      if (!groups.has(r.file_path)) groups.set(r.file_path, []);
      groups.get(r.file_path)!.push(r);
    }
    return [...groups].map(([file, items]) => ({
      file,
      items: items.sort((a, b) => a.start_line - b.start_line),
    }));
  });

  let hasActiveFilters = $derived(activeKinds.size > 0 || activeLangs.size > 0 || activeFiles.size > 0);

  // ── Helpers ──
  function toggle(set: Set<string>, value: string): Set<string> {
    const next = new Set(set);
    if (next.has(value)) next.delete(value); else next.add(value);
    return next;
  }

  function selectResult(name: string) {
    app.activeView = 'explorer';
    app.searchActive = false;
    app.pendingSymbolName = name;
  }

  function resetFilters() {
    activeKinds = new Set();
    activeLangs = new Set();
    activeFiles = new Set();
  }

  function escapeHtml(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  function highlight(text: string, q: string): string {
    if (!q || q.length < 2) return escapeHtml(text);
    const safe = escapeHtml(text);
    const re = new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
    return safe.replace(re, '<mark>$1</mark>');
  }

  function kindAbbrev(kind: string): string {
    return KIND_ABBREV[kind] || kind.substring(0, 2);
  }

  function langColor(lang: string): string {
    return LANG_COLORS[lang] || '#6b7280';
  }

  function formatTime(ms: number): string {
    if (ms < 1000) return `${Math.round(ms)}ms`;
    return `${(ms / 1000).toFixed(2)}s`;
  }
</script>

<div class="search-view">
  {#if !hasSearched}
    <!-- ── Empty State ── -->
    <div class="search-empty">
      <div class="search-hero">
        <svg class="hero-icon" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5">
          <circle cx="20" cy="20" r="14" opacity="0.3"/>
          <circle cx="20" cy="20" r="14"/>
          <line x1="30" y1="30" x2="42" y2="42"/>
        </svg>
        <h1 class="hero-title">Search your codebase</h1>
        <p class="hero-sub">Find functions, classes, types, and symbols across your entire project.</p>
        <div class="filter-hints">
          <div class="filter-hint"><span class="fh-prefix">file:</span> Filter by file path</div>
          <div class="filter-hint"><span class="fh-prefix">kind:</span> Filter by type (Function, Class, Method, Struct...)</div>
          <div class="filter-hint"><span class="fh-prefix">lang:</span> Filter by language (Rust, Python, TypeScript...)</div>
        </div>
        <p class="hero-example">Example: <code>file:parser kind:Function parse</code></p>
      </div>
    </div>

  {:else}
    <!-- ── Results Layout ── -->
    <div class="search-results-layout">
      <!-- Summary bar -->
      <div class="results-summary">
        <span class="results-count">
          {filteredResults.length} result{filteredResults.length !== 1 ? 's' : ''}
          {#if hasActiveFilters}
            <span class="filtered-of">of {results.length}</span>
          {/if}
        </span>
        {#if searchTime !== null}
          <span class="search-timing">in {formatTime(searchTime)}</span>
        {/if}
        {#if results.length >= 100}
          <span class="result-limit-badge">Result limit hit</span>
        {/if}
        {#if hasActiveFilters}
          <button class="reset-btn" onclick={resetFilters}>
            Reset all
            <svg viewBox="0 0 12 12" class="reset-x"><path d="M3 3l6 6M9 3l-6 6" stroke="currentColor" stroke-width="1.5"/></svg>
          </button>
        {/if}

        <!-- Active filter pills (from query prefixes + facet clicks) -->
        <div class="active-pills">
          {#if parsed.file}
            <span class="inline-pill query-filter">file:{parsed.file}</span>
          {/if}
          {#if parsed.kind}
            <span class="inline-pill query-filter">kind:{parsed.kind}</span>
          {/if}
          {#if parsed.lang}
            <span class="inline-pill query-filter lang">lang:{parsed.lang}</span>
          {/if}
          {#each [...activeKinds] as kind}
            <button class="inline-pill" onclick={() => activeKinds = toggle(activeKinds, kind)}>
              <span class="ip-dot" style="background:{KIND_COLORS[kind] || '#6b7280'}"></span>
              {kind}
              <svg viewBox="0 0 12 12" class="ip-x"><path d="M3 3l6 6M9 3l-6 6" stroke="currentColor" stroke-width="1.5"/></svg>
            </button>
          {/each}
          {#each [...activeLangs] as lang}
            <button class="inline-pill lang" onclick={() => activeLangs = toggle(activeLangs, lang)}>
              lang:{lang}
              <svg viewBox="0 0 12 12" class="ip-x"><path d="M3 3l6 6M9 3l-6 6" stroke="currentColor" stroke-width="1.5"/></svg>
            </button>
          {/each}
        </div>
      </div>

      <div class="search-body" class:has-preview={previewFile !== null}>
        <!-- ── Filter Sidebar ── -->
        <aside class="filter-sidebar">
          <!-- By type -->
          {#if kindFacets.length > 0}
            <div class="fs-section">
              <h3>By type</h3>
              {#each kindFacets as f}
                <button
                  class="facet-row"
                  class:active={activeKinds.has(f.kind)}
                  onclick={() => activeKinds = toggle(activeKinds, f.kind)}
                >
                  <span class="facet-icon" style="color:{KIND_COLORS[f.kind] || '#6b7280'}">{@html kindIcon(f.kind, 16)}</span>
                  <span class="facet-label">{f.kind}</span>
                  <span class="facet-count">{f.count}</span>
                </button>
              {/each}
            </div>
          {/if}

          <!-- By language -->
          {#if langFacets.length > 0}
            <div class="fs-section">
              <h3>By language</h3>
              {#each langFacets as f}
                <button
                  class="facet-row"
                  class:active={activeLangs.has(f.lang)}
                  onclick={() => activeLangs = toggle(activeLangs, f.lang)}
                >
                  <span class="facet-icon">{@html langIcon(f.lang, 16)}</span>
                  <span class="facet-label">{f.lang}</span>
                  <span class="facet-count">{f.count}</span>
                </button>
              {/each}
            </div>
          {/if}

          <!-- By file -->
          {#if fileFacets.length > 0}
            <div class="fs-section">
              <h3>By path</h3>
              {#each fileFacets as f}
                <button
                  class="facet-row"
                  class:active={activeFiles.has(f.file)}
                  onclick={() => activeFiles = toggle(activeFiles, f.file)}
                >
                  <span class="facet-icon">{@html fileIcon(f.file.split('/').pop() || f.file, 15)}</span>
                  <span class="facet-label facet-file-label">{f.file}/</span>
                  <span class="facet-count">{f.count}</span>
                </button>
              {/each}
            </div>
          {/if}
        </aside>

        <!-- ── Results ── -->
        <main class="results-main">
          {#if isSearching && results.length === 0}
            <div class="shimmer-wrap">
              {#each Array(6) as _, i}
                <div class="shimmer-group" style="animation-delay: {i * 0.08}s">
                  <div class="shimmer-file"></div>
                  <div class="shimmer-row"></div>
                  <div class="shimmer-row short"></div>
                </div>
              {/each}
            </div>

          {:else if filteredResults.length === 0}
            <div class="no-results">
              <svg class="nr-icon" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5">
                <circle cx="20" cy="20" r="14"/><line x1="30" y1="30" x2="42" y2="42"/>
                <line x1="15" y1="20" x2="25" y2="20"/>
              </svg>
              <p class="nr-title">No symbols found for "{query}"</p>
              {#if hasActiveFilters}
                <p class="nr-hint">Try removing some filters or broadening your search</p>
                <button class="nr-reset" onclick={resetFilters}>Reset filters</button>
              {:else}
                <p class="nr-hint">Try a different query or check the spelling</p>
              {/if}
            </div>

          {:else}
            {#each groupedResults as group, gi}
              <div class="result-group" style="animation-delay: {gi * 0.03}s">
                <div class="rg-header">
                  <span class="rg-file-icon">{@html fileIcon(group.file.split('/').pop() || group.file, 15)}</span>
                  <span class="rg-path">{group.file}</span>
                  <span class="rg-count">{group.items.length}</span>
                  <button
                    class="preview-btn"
                    class:active={previewFilePath === group.file}
                    onclick={() => previewFilePath === group.file ? closePreview() : openPreview(group.file, group.items.map(i => i.start_line))}
                  >
                    Preview
                    <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
                      <path d="M6 3l5 5-5 5"/>
                    </svg>
                  </button>
                </div>
                <div class="rg-items">
                  {#each group.items as result}
                    <button class="result-row" onclick={() => selectResult(result.name)}>
                      <span class="rr-kind" style="color:{KIND_COLORS[result.kind] || '#6b7280'}">
                        {@html kindIcon(result.kind, 16)}
                      </span>
                      <span class="rr-line">{result.start_line}</span>
                      <span class="rr-text">
                        {#if result.signature}
                          {@html highlight(result.signature, parsed.text)}
                        {:else}
                          {@html highlight(result.name, parsed.text)}
                        {/if}
                      </span>
                      {#if result.language}
                        <span class="rr-lang">{@html langIcon(result.language, 14)}</span>
                      {/if}
                    </button>
                  {/each}
                </div>
              </div>
            {/each}
          {/if}
        </main>

        <!-- ── File Preview Panel ── -->
        {#if previewFile || previewLoading}
          <aside class="preview-panel">
            <div class="preview-header">
              <h4 class="preview-title">File Preview</h4>
              {#if previewFile}
                <span class="preview-path">{previewFile.path.split('/').pop()}</span>
              {/if}
              {#if previewMatchLines.length > 0}
                <div class="preview-nav">
                  <button class="pn-btn" onclick={prevMatch} disabled={previewMatchIndex === 0} title="Previous result">
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 3L5 8l5 5"/></svg>
                  </button>
                  <span class="pn-counter">{previewMatchIndex + 1} of {previewMatchLines.length} results</span>
                  <button class="pn-btn" onclick={nextMatch} disabled={previewMatchIndex >= previewMatchLines.length - 1} title="Next result">
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 3l5 5-5 5"/></svg>
                  </button>
                </div>
              {/if}
              <button class="preview-close" onclick={closePreview} title="Close preview">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M4 4l8 8M12 4l-8 8"/>
                </svg>
              </button>
            </div>
            {#if previewLoading}
              <div class="preview-loading">
                <div class="shimmer-row" style="width: 80%"></div>
                <div class="shimmer-row" style="width: 60%"></div>
                <div class="shimmer-row" style="width: 70%"></div>
              </div>
            {:else if previewFile}
              <CodeViewer file={previewFile} highlightLines={currentHighlight} />
            {/if}
          </aside>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  /* ── Layout ── */
  .search-view {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: transparent;
    overflow: hidden;
  }

  /* ── Empty State ── */
  .search-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px 20px;
  }

  .search-hero {
    max-width: 420px;
    width: 100%;
    text-align: center;
  }

  .hero-icon {
    width: 56px;
    height: 56px;
    color: var(--color-text-muted, #4a4a4a);
    margin-bottom: 20px;
  }

  .hero-title {
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 600;
    color: var(--color-text-primary, #e8e8e8);
    margin: 0 0 8px;
    letter-spacing: -0.3px;
  }

  .hero-sub {
    font-family: var(--font-display);
    font-size: 15px;
    color: var(--color-text-secondary, #858585);
    margin: 0 0 20px;
    line-height: 1.5;
  }

  .filter-hints {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0 0 16px;
  }

  .filter-hint {
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--color-text-secondary, #858585);
  }

  .fh-prefix {
    color: var(--color-accent);
    font-weight: 600;
  }

  .hero-example {
    font-family: var(--font-display);
    font-size: 13px;
    color: var(--color-text-muted, #4a4a4a);
    margin: 0;
  }

  .hero-example code {
    font-family: var(--font-mono);
    background: var(--color-surface, #111);
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid var(--color-border, #1e1e1e);
    color: var(--color-text-primary, #e8e8e8);
  }

  .inline-pill.query-filter {
    cursor: default;
    background: rgba(124, 58, 237, 0.12);
    border-color: rgba(124, 58, 237, 0.5);
    color: #a78bfa;
  }

  /* ── Results Layout ── */
  .search-results-layout {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .results-summary {
    padding: 8px 16px;
    border-bottom: 1px solid var(--color-border, #1e1e1e);
    background: transparent;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 36px;
  }

  .results-count {
    font-family: var(--font-display);
    font-size: 14px;
    color: var(--color-text-secondary, #858585);
    flex-shrink: 0;
  }

  .filtered-of {
    color: var(--color-text-muted, #4a4a4a);
  }

  .search-timing {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .result-limit-badge {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    color: var(--color-orange);
    background: rgba(240, 136, 62, 0.12);
    border: 1px solid rgba(240, 136, 62, 0.3);
    border-radius: 4px;
    padding: 1px 8px;
    flex-shrink: 0;
  }

  .reset-btn {
    font-family: var(--font-display);
    font-size: 12px;
    color: var(--color-accent);
    background: none;
    border: none;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px;
    border-radius: 3px;
    transition: background 0.1s;
    flex-shrink: 0;
  }

  .reset-btn:hover {
    background: var(--color-accent-dim);
  }

  .reset-x {
    width: 10px;
    height: 10px;
  }

  .active-pills {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .inline-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px 2px 6px;
    background: var(--color-accent-dim);
    border: 1px solid var(--color-accent);
    border-radius: 4px;
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    transition: background 0.1s;
  }

  .inline-pill:hover {
    background: rgba(7, 119, 255, 0.15);
  }

  .inline-pill.lang {
    background: rgba(49, 120, 198, 0.12);
    border-color: #3178C6;
    color: #6ba7e8;
  }

  .inline-pill.lang:hover {
    background: rgba(49, 120, 198, 0.2);
  }

  .ip-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .ip-x {
    width: 10px;
    height: 10px;
    opacity: 0.6;
    flex-shrink: 0;
  }

  .search-body {
    flex: 1;
    display: flex;
    overflow: hidden;
  }

  /* ── Filter Sidebar ── */
  .filter-sidebar {
    width: 260px;
    border-right: 1px solid var(--color-border, #1e1e1e);
    overflow-y: auto;
    flex-shrink: 0;
    padding: 12px 0;
    background: transparent;
  }

  .fs-section {
    padding: 8px 0 4px;
  }

  .fs-section h3 {
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 600;
    color: var(--color-text-muted, #4a4a4a);
    text-transform: uppercase;
    letter-spacing: 0.8px;
    margin: 0 0 4px;
    padding: 0 14px;
  }

  .facet-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 5px 14px;
    background: none;
    border: none;
    cursor: pointer;
    transition: background 0.08s;
    text-align: left;
  }

  .facet-row:hover {
    background: var(--color-surface-hover, #191919);
  }

  .facet-row.active {
    background: var(--color-accent-dim);
  }

  .facet-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
  }

  .facet-label {
    flex: 1;
    font-family: var(--font-display);
    font-size: 14px;
    color: var(--color-text-primary, #e8e8e8);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .facet-file-label {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-secondary, #858585);
  }

  .facet-count {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--color-text-muted, #4a4a4a);
    flex-shrink: 0;
    min-width: 20px;
    text-align: right;
  }

  /* ── Results Main ── */
  .results-main {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
    min-width: 0;
  }

  /* Loading shimmer */
  .shimmer-wrap {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .shimmer-group {
    animation: shimmerIn 0.4s ease both;
  }

  .shimmer-file {
    height: 16px;
    width: 200px;
    background: var(--color-surface, #111);
    border-radius: 4px;
    margin-bottom: 8px;
    animation: shimmer 1.5s ease-in-out infinite;
  }

  .shimmer-row {
    height: 28px;
    width: 100%;
    background: var(--color-surface, #111);
    border-radius: 4px;
    margin-bottom: 4px;
    animation: shimmer 1.5s ease-in-out infinite;
    animation-delay: 0.15s;
  }

  .shimmer-row.short {
    width: 60%;
    animation-delay: 0.3s;
  }

  /* No results */
  .no-results {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 80px 20px;
    text-align: center;
  }

  .nr-icon {
    width: 48px;
    height: 48px;
    color: var(--color-text-muted, #4a4a4a);
    margin-bottom: 16px;
  }

  .nr-title {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 600;
    color: var(--color-text-primary, #e8e8e8);
    margin: 0 0 6px;
  }

  .nr-hint {
    font-family: var(--font-display);
    font-size: 13px;
    color: var(--color-text-secondary, #858585);
    margin: 0 0 16px;
  }

  .nr-reset {
    font-family: var(--font-display);
    font-size: 12px;
    color: var(--color-accent);
    background: var(--color-accent-dim);
    border: 1px solid var(--color-accent);
    border-radius: 6px;
    padding: 6px 16px;
    cursor: pointer;
    transition: background 0.1s;
  }

  .nr-reset:hover {
    background: rgba(7, 119, 255, 0.15);
  }

  /* Result groups */
  .result-group {
    margin-bottom: 4px;
    border: 1px solid var(--color-border, #1e1e1e);
    border-radius: 8px;
    overflow: hidden;
    animation: fadeSlideIn 0.25s ease both;
  }

  .rg-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    background: var(--color-surface, #111);
    border-bottom: 1px solid var(--color-border, #1e1e1e);
  }

  .rg-file-icon {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .rg-path {
    flex: 1;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--color-text-secondary, #858585);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rg-count {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-text-muted, #4a4a4a);
    background: var(--color-base, #0a0a0a);
    border-radius: 10px;
    padding: 1px 7px;
    flex-shrink: 0;
  }

  .preview-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: var(--font-display);
    font-size: 12px;
    color: var(--color-text-muted);
    background: none;
    border: none;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
    transition: color 0.1s, background 0.1s;
    flex-shrink: 0;
  }

  .preview-btn:hover {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  .preview-btn.active {
    color: var(--color-accent);
    background: var(--color-accent-dim);
  }

  .rg-items {
    background: var(--color-base, #0a0a0a);
  }

  .result-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 5px 12px;
    background: none;
    border: none;
    border-bottom: 1px solid var(--color-border, #1e1e1e);
    cursor: pointer;
    text-align: left;
    transition: background 0.08s;
  }

  .result-row:last-child {
    border-bottom: none;
  }

  .result-row:hover {
    background: var(--color-surface-hover, #191919);
  }

  .rr-kind {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
  }

  .rr-line {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted, #4a4a4a);
    min-width: 32px;
    text-align: right;
    flex-shrink: 0;
  }

  .rr-text {
    flex: 1;
    font-family: var(--font-mono);
    font-size: 14px;
    color: var(--color-text-primary, #e8e8e8);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .rr-text :global(mark) {
    background: rgba(7, 119, 255, 0.2);
    color: var(--color-accent-hover);
    border-radius: 2px;
    padding: 0 1px;
  }

  .rr-lang {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    opacity: 0.85;
  }

  /* ── Preview Panel ── */
  .preview-panel {
    width: 45%;
    min-width: 300px;
    max-width: 600px;
    border-left: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    flex-shrink: 0;
    animation: slideInRight 0.2s ease both;
  }

  .preview-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
    flex-shrink: 0;
    min-height: 38px;
  }

  .preview-title {
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 600;
    color: var(--color-text-primary);
    margin: 0;
    flex-shrink: 0;
  }

  .preview-path {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--color-accent);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .preview-nav {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .pn-btn {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 0;
    transition: color 0.1s, background 0.1s, border-color 0.1s;
  }

  .pn-btn:hover:not(:disabled) {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
    border-color: var(--color-border-bright);
  }

  .pn-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .pn-counter {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    white-space: nowrap;
  }

  .preview-close {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: 4px;
    color: var(--color-text-muted);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }

  .preview-close:hover {
    color: var(--color-text-primary);
    background: var(--color-surface-hover);
  }

  .preview-loading {
    flex: 1;
    padding: 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* ── Animations ── */
  @keyframes shimmer {
    0%, 100% { opacity: 0.3; }
    50% { opacity: 0.6; }
  }

  @keyframes shimmerIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @keyframes fadeSlideIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @keyframes slideInRight {
    from { opacity: 0; transform: translateX(20px); }
    to { opacity: 1; transform: translateX(0); }
  }

  /* ── Responsive ── */
  @media (max-width: 768px) {
    .filter-sidebar {
      display: none;
    }

    .results-main {
      padding: 12px;
    }

    .preview-panel {
      display: none;
    }
  }
</style>
