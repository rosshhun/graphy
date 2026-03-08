# graphy-search

Full-text and fuzzy search for [graphy](https://github.com/rosshhun/graphy).

## Overview

Indexes code graph symbols into a tantivy search index. Supports BM25 ranked search with fuzzy fallback, kind/language/file filtering, and incremental updates.

## Usage

### Write + read (full access)

```rust
use graphy_search::SearchIndex;

// Persistent index (survives restarts)
let index = SearchIndex::new_persistent(&path)?;

// Or in-memory (for tests / ephemeral use)
let index = SearchIndex::new_in_memory()?;

// Index the full graph
index.index_graph(&graph)?;

// Incremental update (only changed files)
index.update_files(&graph, &changed_files)?;

// Search
let results = index.search("parse_config", 10)?;

// Filtered search
let results = index.search_filtered("parse", Some("Function"), Some("Rust"), None, 10)?;
```

### Read-only (safe parallel access)

```rust
use graphy_search::SearchReader;

// Open existing index (no write lock needed)
let reader = SearchReader::open(&path)?;
let results = reader.search("query", 10)?;
```

`SearchReader` is designed for concurrent access — the CLI `search` command uses it so it doesn't conflict with a running `graphy dev` session.

## Search behavior

1. **BM25 ranked search** — Standard full-text search on symbol name, signature, and documentation fields
2. **Fuzzy fallback** — If the query can't be parsed (special characters, typos), falls back to `FuzzyTermQuery` with Levenshtein distance 2
3. **Filter prefixes** — `file:utils`, `kind:Function`, `lang:Rust`

## SearchResult

```rust
pub struct SearchResult {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: u64,
    pub score: f32,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub language: Option<String>,
}
```

## Optional: vector search

Behind the `vectors` feature flag. Uses fastembed-rs (BGE-small-en-v1.5) for semantic similarity search.

```bash
cargo build --features vectors
```

## Dependencies

tantivy 0.22, graphy-core, serde 1, rayon 1
