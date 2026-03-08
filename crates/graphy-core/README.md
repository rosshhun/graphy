# graphy-core

Core graph data structures and persistence layer for [graphy](https://github.com/rosshhun/graphy).

## Overview

This crate defines the **Graphy Intermediate Representation (GIR)** — the language-agnostic schema that all parsers emit and all analysis phases consume. It also provides the graph container, persistence, and diffing.

## Key Types

### GIR Schema

- **`GirNode`** — A node in the code graph (function, class, file, etc.) with metadata: name, kind, file path, span, visibility, signature, documentation, complexity, liveness confidence, coverage.
- **`GirEdge`** — A typed, directed edge (calls, imports, inherits, etc.) with confidence score.
- **`NodeKind`** (20 variants) — Module, File, Folder, Class, Struct, Enum, Interface, Trait, Function, Method, Constructor, Field, Property, Parameter, Variable, Constant, TypeAlias, Import, Decorator, EnumVariant.
- **`EdgeKind`** (17 variants) — Contains, Calls, Imports, ImportsFrom, Inherits, Implements, Overrides, ReturnsType, ParamType, FieldType, Instantiates, DataFlowsTo, TaintedBy, CrossLangCalls, AnnotatedWith, CoupledWith, SimilarTo.
- **`SymbolId`** — FNV-1a hash of `(file_path, name, kind, start_line)`. Deterministic and stable across runs.
- **`Language`** — Python, TypeScript, JavaScript, Rust, Svelte, Go, Java, PHP, C, Cpp, CSharp, Ruby, Kotlin.

### Graph

- **`CodeGraph`** — Wrapper around petgraph `StableGraph` with secondary indexes for O(1) lookup by ID, name, file, or kind. Provides methods for querying callers, callees, children, and edge traversal.
- **`ParseOutput`** — Result of parsing a single file: a list of nodes and edges ready to merge into the graph.

### Persistence

- **`storage`** module — ACID persistence via redb 2 with bincode serialization. SHA-256 checksums for integrity.
  - `save_graph(graph, path)` / `load_graph(path)`
  - `default_db_path(project_root)` — Returns `.graphy/graph.redb`

### Diff

- **`diff`** module — Structural diff between two graph versions. Detects added/removed/modified symbols and breaking changes (removed public symbols, changed signatures, narrowed visibility). Used by `graphy diff` for CI/CD gating.

## Dependencies

petgraph 0.6, redb 2, bincode 1, serde 1, sha2 0.10, anyhow 1, thiserror 2
