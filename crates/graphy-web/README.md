# graphy-web

Web dashboard and REST API for [graphy](https://github.com/rosshhun/graphy).

## Overview

Axum-based REST API serving graph data + an embedded Svelte 5 single-page application. The frontend is compiled into the binary via `rust-embed`, so no separate web server is needed.

## Usage

```rust
use graphy_web::{AppState, serve};

let state = AppState {
    graph: Arc::new(RwLock::new(graph)),
    search: Arc::new(search_index),
    project_root: root,
};

let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
serve(state, 3000, shutdown_rx).await?;
```

## REST API

| Endpoint | Description |
|----------|-------------|
| `GET /api/stats` | Node/edge/file/function/class counts |
| `GET /api/search?q=&kind=&lang=&file=` | Full-text search with filtering |
| `GET /api/symbol/:name` | Symbol detail (complexity, callers, callees, children) |
| `GET /api/graph` | Graph data for visualization (top 400 nodes + edges) |
| `GET /api/files` | List of all indexed source files |
| `GET /api/hotspots` | Complexity hotspots ranked by risk |
| `GET /api/dead-code` | Dead code findings with liveness scores |
| `GET /api/taint` | Taint analysis paths |
| `GET /api/architecture` | File/language distribution, largest files |
| `GET /api/patterns` | Code smell patterns |
| `GET /api/api-surface` | Public vs internal API breakdown |
| `GET /api/file-content?path=` | Raw file content |
| `GET /api/file-symbols?path=` | Symbols in a specific file |

## Frontend

The Svelte 5 frontend lives in the `web/` directory at the workspace root. It's built with Vite and embedded into the binary at compile time.

**4 views:**
- **Explorer** — Force-directed graph (Sigma.js + Graphology), file tree, symbol detail panel
- **Analysis** — Health score, complexity hotspots, dead code, anti-patterns
- **Security** — Taint paths, public API exposure
- **Architecture** — Language distribution, node/edge breakdown, largest files

### Development

```bash
# Terminal 1: Start backend
graphy open ./your-project --port 3000

# Terminal 2: Start Vite dev server with hot reload
cd web && npm run dev
# Frontend at localhost:5173, proxies /api to :3000
```

## Optional feature

This crate is gated behind the `web` feature in the CLI. Install without it for a smaller binary:

```bash
cargo install graphy --no-default-features
```

## Dependencies

axum 0.7, tower-http 0.5 (CORS), rust-embed 8, graphy-core, graphy-search, serde_json 1, tokio 1
