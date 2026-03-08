# graphy-analysis

15-phase analysis pipeline for [graphy](https://github.com/rosshhun/graphy).

## Overview

Takes a parsed code graph and enriches it through 15 sequential analysis phases — from import resolution to dead code detection. All phases are language-agnostic, operating on GIR nodes and edges.

## Pipeline

```rust
let pipeline = AnalysisPipeline::new(project_root, config);
let graph = pipeline.run()?;
```

| Phase | Module | Description |
|-------|--------|-------------|
| 1 | `discovery` | Walk directory, respect `.gitignore`, hash files |
| 2 | `structure` | Build file/folder/module hierarchy |
| 3 | (parser) | Tree-sitter parsing (parallel via rayon) |
| 4 | `import_resolution` | Resolve imports to target files/symbols |
| 5 | `call_tracing` | Map calls to definitions with type-aware disambiguation |
| 5.5 | (cleanup) | Remove orphaned phantom nodes |
| 5.7 | `lsp_enhance` | Optional: query rust-analyzer/pyright/gopls for precise resolution |
| 5.8 | `framework` | TOML-driven framework detection and annotation |
| 6 | `heritage` | BFS inheritance chains, Overrides edges |
| 7 | `type_analysis` | Resolve type annotations to real type nodes |
| 8 | `dataflow` | Positional parameter matching at call sites |
| 9 | `taint` | BFS source-to-sink propagation with sanitizers |
| 10 | `complexity` | Cyclomatic, cognitive, LOC, nesting depth |
| 11 | `community` | Label propagation on call graph |
| 12 | `flow_detection` | BFS from entry points (main, routes, tests) |
| 13 | `dead_code` | Probabilistic liveness scoring with 13 heuristics |
| 14 | `change_coupling` | Git co-change frequency with temporal decay |

## Configuration

```rust
let config = PipelineConfig {
    incremental: true,   // Skip unchanged files
    use_lsp: false,      // Enable LSP enhancement
    ..Default::default()
};
```

## Key modules

### Framework detection (`framework`)

TOML-driven plugin system. 15 built-in plugins (Express, Django, Laravel, React, etc.) + user-defined plugins at `~/.config/graphy/frameworks/*.toml`.

Detects frameworks via lockfile dependencies, imports, or file conventions. Creates `AnnotatedWith` edges and boosts liveness scores for framework-managed functions.

### Taint analysis (`taint`)

Language-specific source/sink/sanitizer patterns:

- **Python:** `request.args` to `cursor.execute`, sanitized by `html.escape`
- **PHP:** `get_option` to `query`, sanitized by `esc_html`/`prepare`
- **JS/TS:** `req.body` to `eval`/`innerHTML`, sanitized by `DOMPurify`
- **Rust:** `env::var` to `Command::new`, sanitized by `bind`

Custom rules via `.graphy/taint.toml`.

### Dead code detection (`dead_code`)

Probabilistic liveness scoring (0.0 = dead, 1.0 = alive) using 13 heuristics: entry_point, decorated, method_on_used_type, trait_impl, dunder, constructor, public_api, callers, overrides, `__all__`, string_reference, identifier_reference, svelte_component.

### LSP enhancement (`lsp_enhance`)

Optional integration with language servers for precise call resolution. Supports rust-analyzer, pyright, typescript-language-server, gopls. Pipelined requests (chunks of 50) for performance.

### Additional modules

- **`context_gen`** — Generate codebase context documents (markdown/JSON)
- **`coverage`** — Parse lcov files and overlay coverage data on graph nodes
- **`multi_repo`** — Analyze multiple repositories as a unified graph

## Dependencies

graphy-core, graphy-parser, petgraph 0.6, rayon 1, regex 1, toml 0.8
