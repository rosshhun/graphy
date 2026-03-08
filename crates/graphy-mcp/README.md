# graphy-mcp

MCP server for [graphy](https://github.com/rosshhun/graphy).

## Overview

Implements the [Model Context Protocol](https://modelcontextprotocol.io/) over JSON-RPC 2.0 / stdio. Exposes the code graph to AI agents like Claude Code through 3 consolidated tools and 3 resources.

## Usage

### Standalone

```rust
use graphy_mcp::McpServer;

let server = McpServer::new(graph, Some(search_index), project_root);
server.run().await?;
```

### With watch mode (shared graph)

```rust
use graphy_mcp::{McpServer, notification_channel};

let graph = Arc::new(RwLock::new(graph));
let (notify_tx, notify_rx) = notification_channel();

let server = McpServer::new_shared(graph.clone(), search, root)
    .with_notifications(notify_rx);

// Server reads from the shared graph on each tool call
// Watcher updates the graph in the background
// Notifications sent via notify_tx trigger `notifications/resources/updated`
```

## Tools

| Tool | Mode | Description |
|------|------|-------------|
| `graphy_query` | `search` | Hybrid BM25 + fuzzy symbol search |
| | `context` | Full context with callers, callees, source snippets |
| | `explain` | Deep explanation with complexity, liveness, full source |
| | `file` | All symbols in a file with external callers |
| `graphy_analyze` | `dead_code` | Unused code with liveness probability |
| | `hotspots` | Complexity x coupling risk ranking |
| | `architecture` | Module overview, language breakdown |
| | `patterns` | Anti-patterns (god classes, long params, deep nesting) |
| | `api_surface` | Public vs internal symbol classification |
| | `deps` | Dependency tree with vulnerability check |
| `graphy_trace` | `impact` | Blast radius from a symbol |
| | `taint` | Source-to-sink data flow paths |
| | `dataflow` | Data transformation chains |
| | `tests` | Tests exercising a symbol via call graph |

All tool responses include source code snippets at call sites and a graph confidence footer (call coverage %, import resolution %).

**Batch queries:** `graphy_query` accepts a `queries` array for multi-symbol lookup in one call.

## Resources

| URI | Description |
|-----|-------------|
| `graphy://architecture` | File count, languages, largest modules, entry points |
| `graphy://security` | Taint paths, public API exposure |
| `graphy://health` | Dead code %, complexity hotspots, graph confidence |

## Protocol

- JSON-RPC 2.0 over stdio (manual implementation, no external MCP SDK)
- Protocol version: `2024-11-05`
- Capabilities: `tools`, `resources`
- All logging via `tracing` (never writes to stdout, which would corrupt the protocol)

## Session context

Tracks explored symbols across calls within a session. Suggests related unexplored neighbors to guide the AI toward comprehensive understanding.

## Dependencies

graphy-core, graphy-search, graphy-deps, tokio 1, serde_json 1
