# graphy-watch

File watcher and incremental re-indexing for [graphy](https://github.com/rosshhun/graphy).

## Overview

Watches a project directory for file changes and incrementally re-indexes only the affected files. Uses lock-free parsing (parse outside the graph write lock) for minimal disruption.

## Usage

```rust
use graphy_watch::FileWatcher;

let graph = Arc::new(RwLock::new(graph));

let watcher = FileWatcher::new(root, graph.clone())
    .with_search(search_index)       // Update search index on changes
    .with_lsp(true)                  // Use LSP for precise resolution
    .with_on_reindex(Box::new(|files, nodes, edges| {
        // Called after each re-index (e.g., send MCP notification)
    }));

watcher.watch().await?;
```

## How it works

1. **File watching** — `notify-debouncer-full` with 200ms debounce
2. **Change detection** — SHA-256 content hashing (ignores timestamp-only changes)
3. **Parse phase** (outside lock) — Re-parse changed files in parallel via rayon
4. **Mutate phase** (inside lock) — Remove old nodes, insert new nodes, re-run import resolution + call tracing + complexity + dead code
5. **Search update** (outside lock) — Incrementally update tantivy index for changed files

### Two-phase locking

```
File change detected
    |
    v
Parse changed files (rayon, no lock held)
    |
    v
Acquire write lock
    |-- Remove old nodes for changed files
    |-- Insert new nodes
    |-- Re-resolve imports and calls
    |-- Re-run complexity and dead code
    |
    v
Release write lock
    |
    v
Update search index (no lock held)
    |
    v
Fire on_reindex callback
```

## Features

- **Incremental dead code** — Liveness scores stay fresh after partial re-index
- **Warm LSP** — Persistent `LspClient` across re-indexes, sends `didChange` notifications
- **Search sync** — Search index updated for changed files only
- **Notification callback** — Hook for MCP server to send `resources/updated` notifications

## Dependencies

graphy-core, graphy-parser, graphy-analysis, graphy-search, notify-debouncer-full 0.3, sha2 0.10, tokio 1
