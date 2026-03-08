# Graphy

Graph-powered code intelligence engine. Indexes codebases into a knowledge graph exposed via MCP tools + web UI.

## Quick Start

```bash
# CLI-only install (for AI agents / MCP server) — 18MB binary
cargo install --path crates/graphy-cli --no-default-features

# Full install (includes web dashboard) — 29MB binary
cd web && npm run build && cd ..
cargo install --path crates/graphy-cli

# Install language grammars you need
graphy lang add go java php

# Run on a project
graphy              # analyze + watch (CLI-only) or analyze + dashboard + watch (full)
graphy analyze .    # index only
graphy serve --watch .  # MCP server with live re-index (for Claude Code)
graphy dev .        # analyze + dashboard on :3000 + file watcher (requires web feature)
```

## Project Structure

```
graphy/
├── crates/                     # 9 Rust crates (Cargo workspace)
│   ├── graphy-core/           # GIR schema, petgraph graph, redb persistence, graph diff
│   ├── graphy-parser/         # tree-sitter parsing, dynamic grammar loading
│   ├── graphy-analysis/       # 14-phase pipeline, dead code, taint, complexity
│   ├── graphy-search/         # tantivy BM25 + fuzzy search + optional vectors
│   ├── graphy-mcp/            # MCP server (JSON-RPC over stdio), 14 tools
│   ├── graphy-watch/          # File watcher + incremental re-index
│   ├── graphy-web/            # axum REST API + rust-embed Svelte frontend
│   ├── graphy-deps/           # Lockfile parsers + OSV.dev vulnerability queries
│   └── graphy-cli/            # clap CLI, binary name `graphy`, 15 commands
├── config/                     # External config files
│   ├── frameworks/             # Framework plugin TOML configs (15 built-in)
│   └── tags/                   # Tree-sitter tag queries (.scm files, bundled fallback)
├── web/                        # Svelte 5 + TypeScript + Tailwind CSS v4 frontend
└── .github/workflows/          # CI/CD
```

## Architecture

### Analysis Pipeline (14 phases, sequential)

```
Phase 1:  File Discovery        — walk + hash files
Phase 2:  Structure Building    — File/Folder/Module hierarchy
Phase 3:  AST Parsing           — tree-sitter (parallel via rayon)
Phase 4:  Import Resolution     — Python dotted paths, JS relative, Rust crate map
Phase 5:  Call Tracing           — resolve phantom call targets to real definitions
Phase 5.5: Phantom Cleanup      — remove orphaned phantoms
Phase 5.7: LSP Enhancement      — optional, queries rust-analyzer/pyright/gopls
Phase 5.8: Framework Detection  — TOML-driven plugins, creates AnnotatedWith edges
Phase 6:  Heritage Analysis     — BFS inheritance chains, Overrides edges
Phase 7:  Type Analysis         — resolve type annotations to real types
Phase 8:  Data Flow             — positional parameter matching at call sites
Phase 9:  Taint Analysis        — BFS source→sink propagation
Phase 10: Complexity Metrics    — cyclomatic, cognitive, LOC, nesting depth
Phase 11: Community Detection   — label propagation on call graph
Phase 12: Flow Detection        — BFS from entry points (main, routes, tests)
Phase 13: Dead Code Detection   — 13 heuristics, probabilistic liveness scoring
Phase 14: Change Coupling       — git co-change frequency analysis
```

### Graph Model

- **Library:** petgraph StableGraph (directed multigraph)
- **Node types (20):** File, Folder, Module, Class, Struct, Enum, Interface, Trait, Function, Method, Constructor, Field, Property, Parameter, Variable, Constant, TypeAlias, Import, Decorator, EnumVariant
- **Edge types (17):** Contains, Calls, Imports, ImportsFrom, Inherits, Implements, Overrides, ReturnsType, ParamType, FieldType, Instantiates, DataFlowsTo, TaintedBy, CrossLangCalls, AnnotatedWith, CoupledWith, SimilarTo
- **SymbolId:** FNV-1a hash of (file_path, name, kind, start_line)
- **Secondary indexes:** 4 HashMaps (id, name, file, kind) for O(1) lookups
- **Persistence:** redb 2 (ACID), bincode serialization, SHA-256 checksums

### Language Support

**Built-in (always available):**
- Python, TypeScript/JavaScript, Rust, Svelte — custom tree-sitter frontends

**Dynamic (install with `graphy lang add <name>`):**
- Go, Java, PHP, C, C++, C#, Ruby, Kotlin
- Grammars compiled from source using system C compiler
- Stored at `~/.config/graphy/grammars/<lang>/`
- Uses `libloading` to load `.so`/`.dylib` at runtime (same approach as Neovim)
- Pinned to tree-sitter ABI 14 compatible commits

### Framework Plugins (TOML-driven)

15 built-in frameworks defined as TOML files in `config/frameworks/`:
WordPress, React, Next.js, Express, Flask, Django, FastAPI, Laravel, Spring Boot, Rails, Angular, Vue, NestJS, SvelteKit, Axum

Custom frameworks can be added at `~/.config/graphy/frameworks/` (loaded automatically).

### Search

- **BM25:** tantivy full-text search on name, signature, doc fields
- **Fuzzy fallback:** FuzzyTermQuery (edit distance 2) when BM25 parse fails
- **Vector search:** fastembed-rs (BGE-small-en-v1.5) behind `vectors` feature flag

### MCP Server

3 consolidated tools over JSON-RPC/stdio (manual implementation, no rmcp dependency):
graphy_query (modes: search/context/explain/file), graphy_analyze (modes: dead_code/hotspots/architecture/patterns/api_surface/deps), graphy_trace (modes: impact/taint/dataflow/tests). All responses include source code snippets at call sites. Taint analysis uses language-specific source/sink/sanitizer patterns (Python, PHP/WordPress, JS/TS, Rust).

### Watch Mode

- notify-debouncer-full (200ms debounce)
- SHA-256 content hashing for change detection
- Two-phase locking: parse outside lock, mutate inside lock
- Warm LSP persistence across re-indexes

## Build & Install

```bash
# CLI-only (for AI agents — no web dashboard, smaller binary)
cargo install --path crates/graphy-cli --no-default-features

# Full (includes web dashboard)
cd web && npm run build && cd ..
cargo install --path crates/graphy-cli

# Install dynamic grammars
graphy lang add go java php c cpp c-sharp ruby
```

### Feature Flags

| Feature | Default | Description |
|---|---|---|
| `web` | yes | Embeds Svelte dashboard, enables `graphy dev` and `graphy open` commands |
| `vectors` | no | Enables fastembed semantic search (adds ~100MB to binary) |

### Resource Usage

| Metric | CLI-only | With web |
|---|---|---|
| Binary size | 18 MB | 29 MB |
| Threads (`serve --watch`) | 7 | 7 |
| Threads (`dev`) | n/a | 9 |
| RSS (2500 symbols) | ~50 MB | ~50 MB |

Thread budget: 2 Tokio workers + 2 Rayon + 2 notify + 1 main. Web adds 2 threads (web server + search).
Uses jemalloc allocator and SingleSegmentIndexWriter (zero-thread tantivy writes).

## Testing

```bash
cargo test --workspace     # 100 tests across 9 crates
```

## Key Dependencies

petgraph 0.6, redb 2, bincode 1, tree-sitter 0.24, tantivy 0.22, clap 4, tokio 1, rayon 1, axum 0.7, libloading 0.8, sha2 0.10, toml 0.8, regex 1

## CLI Commands

| Command | Description |
|---|---|
| `graphy` | Analyze + dashboard + watch (default) |
| `graphy dev [path]` | Same as above, explicit |
| `graphy init` | Create `.mcp.json` for Claude Code |
| `graphy open [path]` | Open dashboard (skip re-analysis) |
| `graphy analyze [path]` | Index a repository |
| `graphy search <query>` | Search for symbols |
| `graphy context <symbol>` | Full symbol context |
| `graphy impact <symbol>` | Blast radius analysis |
| `graphy dead-code [path]` | Report dead code |
| `graphy hotspots` | Complexity hotspots |
| `graphy taint [symbol]` | Taint analysis results |
| `graphy stats [path]` | Graph statistics |
| `graphy serve [path]` | Start MCP server |
| `graphy watch [path]` | File watcher mode |
| `graphy deps [path]` | Dependencies + vulns |
| `graphy diff <base> <head>` | Graph diff (CI/CD) |
| `graphy context-gen [path]` | Generate context doc |
| `graphy lang add <name>` | Install a language grammar |
| `graphy lang remove <name>` | Remove a language grammar |
| `graphy lang list` | List available/installed grammars |
| `graphy multi-repo <paths>` | Multi-repo analysis |

## Design Principles

1. **One parser (tree-sitter)** — Built-in for core languages, dynamic loading for everything else
2. **Data over code** — Framework plugins and tag queries are config files, not Rust code
3. **Lock-free parsing** — Watch mode parses outside the graph write lock
4. **Single source of truth** — Dead code confidence stored in `node.confidence`, read by CLI/MCP/Web
5. **Incremental everything** — File hashing, scoped re-parsing, targeted search updates
