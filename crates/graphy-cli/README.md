# graphy

CLI entry point for [graphy](https://github.com/rosshhun/graphy) — a graph-powered code intelligence engine.

This is the main binary crate. Install it with:

```bash
# CLI only (for AI agents / MCP)
cargo install graphy --no-default-features

# With web dashboard
cargo install graphy
```

## Commands

| Command | Description |
|---------|-------------|
| `graphy` | Analyze + dashboard + watch (default, requires `web` feature) |
| `graphy dev` | Same as above |
| `graphy init` | Create `.mcp.json` for Claude Code |
| `graphy open` | Open dashboard only (requires `web` feature) |
| `graphy analyze [--full] [--lsp]` | Index a repository |
| `graphy search <query>` | Search for symbols |
| `graphy context <symbol>` | Full symbol context |
| `graphy impact <symbol>` | Blast radius analysis |
| `graphy dead-code` | Report dead code |
| `graphy hotspots` | Complexity hotspots |
| `graphy taint [symbol]` | Taint analysis |
| `graphy stats` | Graph statistics |
| `graphy serve [--watch] [--lsp]` | Start MCP server |
| `graphy watch [--lsp]` | File watcher mode |
| `graphy deps [--vulns]` | Dependency analysis |
| `graphy diff <base> <head>` | Graph diff (CI/CD) |
| `graphy context-gen` | Generate context document |
| `graphy lang add\|remove\|list` | Manage language grammars |
| `graphy multi-repo <paths>` | Multi-repo analysis |

## Feature flags

| Feature | Default | Effect |
|---------|---------|--------|
| `web` | on | Enables `dev`, `open` commands and web dashboard |

## Runtime configuration

- **Tokio:** 2 worker threads + 2 blocking threads
- **Rayon:** 2 threads for parallel parsing
- **Allocator:** jemalloc (reduces memory fragmentation)
- **Tantivy:** SingleSegmentIndexWriter (zero background threads)

## Dependencies

All 8 graphy crates + clap 4, tokio 1, rayon 1, tikv-jemallocator 0.6
