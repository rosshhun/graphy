<script lang="ts">
  import type { FileContent } from '../api/client.js';
  import { fileIcon } from '../icons.js';
  import { createHighlighter, type Highlighter, type ThemedToken } from 'shiki';

  let {
    file,
    highlightLines = [],
    onLineClick,
  }: {
    file: FileContent;
    highlightLines?: number[];
    onLineClick?: (line: number) => void;
  } = $props();

  let lines = $derived(file.content.split('\n'));
  let scrollContainer = $state<HTMLElement | undefined>(undefined);
  let highlightSet = $derived(new Set(highlightLines));

  // Shiki syntax highlighting
  let tokenLines = $state<ThemedToken[][] | null>(null);
  let highlighterReady = $state(false);

  const LANG_MAP: Record<string, string> = {
    rs: 'rust', ts: 'typescript', tsx: 'tsx', js: 'javascript', jsx: 'jsx',
    py: 'python', go: 'go', java: 'java', php: 'php',
    c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', hpp: 'cpp',
    cs: 'csharp', rb: 'ruby', svelte: 'svelte',
    json: 'json', toml: 'toml', yaml: 'yaml', yml: 'yaml',
    md: 'markdown', html: 'html', css: 'css', scss: 'scss',
    sh: 'bash', bash: 'bash', zsh: 'bash',
    xml: 'xml', sql: 'sql', graphql: 'graphql',
    dockerfile: 'dockerfile', makefile: 'makefile',
  };

  const THEME = 'github-dark-default';
  const ALL_LANGS = [...new Set(Object.values(LANG_MAP))];

  let highlighterPromise: Promise<Highlighter> | null = null;

  function getHighlighter(): Promise<Highlighter> {
    if (!highlighterPromise) {
      highlighterPromise = createHighlighter({
        themes: [THEME],
        langs: ALL_LANGS,
      });
    }
    return highlighterPromise;
  }

  function getLang(path: string): string | null {
    const ext = path.split('/').pop()?.split('.').pop()?.toLowerCase() || '';
    const name = path.split('/').pop()?.toLowerCase() || '';
    if (name === 'dockerfile') return 'dockerfile';
    if (name === 'makefile') return 'makefile';
    return LANG_MAP[ext] || null;
  }

  $effect(() => {
    const code = file.content;
    const lang = getLang(file.path);
    tokenLines = null;

    if (!lang) {
      highlighterReady = true;
      return;
    }

    getHighlighter().then((hl) => {
      const result = hl.codeToTokens(code, { lang, theme: THEME });
      tokenLines = result.tokens;
      highlighterReady = true;
    }).catch(() => {
      highlighterReady = true;
    });
  });

  $effect(() => {
    if (highlightLines.length > 0 && scrollContainer) {
      const lineEl = scrollContainer.querySelector(`[data-line="${highlightLines[0]}"]`);
      if (lineEl) {
        lineEl.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }
    }
  });

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="code-viewer">
  <div class="cv-header">
    <div class="cv-breadcrumbs">
      <span class="bc-icon">{@html fileIcon(file.path.split('/').pop() || file.path, 15)}</span>
      {#each file.path.split('/') as part, i}
        {#if i > 0}<span class="bc-sep">/</span>{/if}
        <span class="bc-part" class:bc-last={i === file.path.split('/').length - 1}>{part}</span>
      {/each}
    </div>
    <span class="cv-meta">{file.line_count} lines &middot; {formatSize(file.size_bytes)}</span>
  </div>

  <div class="cv-code" bind:this={scrollContainer}>
    <table class="code-table">
      <tbody>
        {#each lines as line, i}
          {@const lineNum = i + 1}
          {@const tokens = tokenLines?.[i]}
          <tr
            class="code-line"
            class:highlighted={highlightSet.has(lineNum)}
            data-line={lineNum}
            onclick={() => onLineClick?.(lineNum)}
          >
            <td class="line-number">{lineNum}</td>
            <td class="line-content">
              <pre>{#if tokens}{#each tokens as tok}<span style="color:{tok.color}">{tok.content}</span>{/each}{:else}{line}{/if}</pre>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .code-viewer {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .cv-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 16px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    gap: 12px;
    min-height: 38px;
  }

  .cv-breadcrumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--color-text-secondary);
    overflow: hidden;
    white-space: nowrap;
    min-width: 0;
  }

  .bc-icon {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    margin-right: 4px;
  }

  .bc-sep {
    color: var(--color-text-muted);
  }

  .bc-part {
    color: var(--color-text-secondary);
  }

  .bc-last {
    color: var(--color-text-primary);
    font-weight: 500;
  }

  .cv-meta {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
    flex-shrink: 0;
    white-space: nowrap;
  }

  .cv-code {
    flex: 1;
    overflow: auto;
    background: var(--color-base);
  }

  .code-table {
    border-collapse: collapse;
    width: 100%;
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 20px;
  }

  .code-line {
    cursor: pointer;
    transition: background 0.06s;
  }

  .code-line:hover {
    background: var(--color-surface-hover);
  }

  .code-line.highlighted {
    background: rgba(7, 119, 255, 0.08);
  }

  .code-line.highlighted .line-number {
    color: var(--color-accent);
    border-right-color: var(--color-accent);
  }

  .line-number {
    width: 1px;
    white-space: nowrap;
    padding: 0 16px;
    text-align: right;
    color: var(--color-text-muted);
    user-select: none;
    border-right: 1px solid var(--color-border);
    vertical-align: top;
    font-size: 12px;
  }

  .line-content {
    padding: 0 16px;
    white-space: pre;
  }

  .line-content pre {
    margin: 0;
    font-family: inherit;
    font-size: inherit;
    line-height: inherit;
  }
</style>
