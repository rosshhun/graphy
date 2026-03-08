use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use serde_json::{json, Value};

use graphy_core::{CodeGraph, EdgeKind, GirNode, NodeKind, SymbolId, Visibility};
use graphy_search::SearchIndex;

use crate::protocol::{CallToolResult, ToolDefinition};

// ── Session Context ─────────────────────────────────────────

/// Tracks symbols queried across MCP calls within a session.
/// Enables "related unexplored" hints to guide Claude's exploration.
pub struct SessionContext {
    /// Symbols that have been explicitly queried (via context/explain/impact/etc.)
    explored: Mutex<HashSet<SymbolId>>,
    /// Map from symbol ID to name (for display)
    explored_names: Mutex<HashMap<SymbolId, String>>,
}

impl SessionContext {
    pub fn new() -> Self {
        Self {
            explored: Mutex::new(HashSet::new()),
            explored_names: Mutex::new(HashMap::new()),
        }
    }

    /// Record that a symbol was queried.
    pub fn record(&self, id: SymbolId, name: &str) {
        self.explored.lock().unwrap().insert(id);
        self.explored_names
            .lock()
            .unwrap()
            .insert(id, name.to_string());
    }

    /// Record multiple symbols from a tool call.
    pub fn record_nodes(&self, nodes: &[&GirNode]) {
        let mut explored = self.explored.lock().unwrap();
        let mut names = self.explored_names.lock().unwrap();
        for node in nodes {
            explored.insert(node.id);
            names.insert(node.id, node.name.clone());
        }
    }

    /// Find connected symbols that haven't been explored yet.
    fn unexplored_neighbors(&self, graph: &CodeGraph) -> Vec<(String, String)> {
        let explored = self.explored.lock().unwrap();
        if explored.is_empty() {
            return Vec::new();
        }

        let mut suggestions: HashMap<SymbolId, (String, String)> = HashMap::new();

        for &id in explored.iter() {
            // Callers not yet explored
            for caller in graph.callers(id) {
                if !explored.contains(&caller.id) && caller.kind.is_callable() {
                    suggestions.entry(caller.id).or_insert_with(|| {
                        (
                            caller.name.clone(),
                            format!("calls explored symbol"),
                        )
                    });
                }
            }
            // Callees not yet explored
            for callee in graph.callees(id) {
                if !explored.contains(&callee.id) && callee.kind.is_callable() {
                    suggestions.entry(callee.id).or_insert_with(|| {
                        (
                            callee.name.clone(),
                            format!("called by explored symbol"),
                        )
                    });
                }
            }
        }

        let mut result: Vec<_> = suggestions.into_values().collect();
        result.truncate(5);
        result
    }

    fn explored_count(&self) -> usize {
        self.explored.lock().unwrap().len()
    }
}

/// Return the 3 consolidated tool definitions.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "graphy_query".into(),
            description: concat!(
                "Search and explore code. Supports batch queries via 'queries' array. ",
                "mode=search: find symbols by name/keyword with fuzzy matching. ",
                "mode=context: show callers, callees, types with source snippets. ",
                "mode=explain: deep explanation with full source code, complexity, liveness. ",
                "mode=file: list all symbols in a file and their external callers with source.",
            )
            .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name, search terms, or file path (for mode=file)" },
                    "queries": { "type": "array", "items": { "type": "string" }, "description": "Batch: multiple symbol names to look up at once (uses same mode for all)" },
                    "mode": { "type": "string", "enum": ["search", "context", "explain", "file"], "default": "search" },
                    "max_results": { "type": "integer", "default": 20 },
                    "kind": { "type": "string", "description": "Filter by kind: Function, Class, Method, Struct, etc." }
                }
            }),
        },
        ToolDefinition {
            name: "graphy_analyze".into(),
            description: concat!(
                "Run codebase analysis. ",
                "dead_code: find unused functions with liveness scores and source preview. ",
                "hotspots: rank code by complexity * coupling risk. ",
                "architecture: structural overview with module sizes. ",
                "patterns: detect anti-patterns (god classes, high complexity). ",
                "api_surface: classify public vs internal symbols. ",
                "deps: dependency tree with optional vulnerability check.",
            )
            .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "analysis": { "type": "string", "enum": ["dead_code", "hotspots", "architecture", "patterns", "api_surface", "deps"] },
                    "max_results": { "type": "integer", "default": 20 },
                    "file_path": { "type": "string", "description": "Limit to a specific file" },
                    "check_vulns": { "type": "boolean", "description": "For deps: check OSV.dev for vulnerabilities", "default": false },
                    "detail_level": { "type": "string", "enum": ["summary", "normal", "verbose"], "default": "normal" }
                },
                "required": ["analysis"]
            }),
        },
        ToolDefinition {
            name: "graphy_trace".into(),
            description: concat!(
                "Trace relationships through the code graph. ",
                "impact: blast radius of changing a symbol, with source at each call site. ",
                "taint: security data flow from sources (user input) to sinks (SQL/shell/eval). ",
                "dataflow: data transformation chains through function calls. ",
                "tests: find test functions that exercise a symbol via call graph.",
            )
            .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name (required for impact/dataflow/tests, optional for taint)" },
                    "mode": { "type": "string", "enum": ["impact", "taint", "dataflow", "tests"] },
                    "max_depth": { "type": "integer", "default": 5 }
                },
                "required": ["mode"]
            }),
        },
    ]
}

/// Dispatch a single query by mode.
fn dispatch_query(
    mode: &str,
    args: &Value,
    graph: &CodeGraph,
    search: Option<&SearchIndex>,
    session: &SessionContext,
) -> CallToolResult {
    let mut a = args.clone();
    match mode {
        "context" => {
            if a["symbol"].is_null() {
                a["symbol"] = a["query"].clone();
            }
            let r = tool_context(&a, graph);
            if let Some(sym) = a["symbol"].as_str() {
                for node in graph.find_by_name(sym) {
                    session.record(node.id, &node.name);
                }
            }
            r
        }
        "explain" => {
            if a["symbol"].is_null() {
                a["symbol"] = a["query"].clone();
            }
            let r = tool_explain(&a, graph);
            if let Some(sym) = a["symbol"].as_str() {
                for node in graph.find_by_name(sym) {
                    session.record(node.id, &node.name);
                }
            }
            r
        }
        "file" => {
            if a["file_path"].is_null() {
                a["file_path"] = a["query"].clone();
            }
            tool_changes(&a, graph)
        }
        _ => tool_search(&a, graph, search),
    }
}

/// Dispatch a tool call to its handler.
pub fn handle_tool(
    name: &str,
    args: &Value,
    graph: &CodeGraph,
    search: Option<&SearchIndex>,
    project_root: &Path,
    session: &SessionContext,
) -> CallToolResult {
    let result = match name {
        "graphy_query" => {
            let mode = args["mode"].as_str().unwrap_or("search");

            // Batch support: if "queries" array is provided, run each query and combine
            if let Some(queries) = args["queries"].as_array() {
                let mut combined = String::new();
                for (i, q) in queries.iter().enumerate() {
                    if let Some(q_str) = q.as_str() {
                        let mut a = args.clone();
                        a["query"] = Value::String(q_str.to_string());
                        let single = dispatch_query(mode, &a, graph, search, session);
                        if i > 0 {
                            combined.push_str("\n---\n\n");
                        }
                        combined.push_str(&format!("## Query: `{q_str}`\n\n"));
                        if let Some(block) = single.content.first() {
                            combined.push_str(&block.text);
                        }
                    }
                }
                if combined.is_empty() {
                    CallToolResult::error("'queries' array is empty or contains no strings".into())
                } else {
                    CallToolResult::text(combined)
                }
            } else if args["query"].is_null() || args["query"].as_str().map_or(true, |s| s.is_empty()) {
                CallToolResult::error("Provide 'query' (string) or 'queries' (array of strings)".into())
            } else {
                dispatch_query(mode, args, graph, search, session)
            }
        }
        "graphy_analyze" => {
            let analysis = args["analysis"].as_str().unwrap_or("");
            match analysis {
                "dead_code" => tool_dead_code(args, graph),
                "hotspots" => tool_hotspots(args, graph),
                "architecture" => tool_architecture(args, graph),
                "patterns" => tool_patterns(args, graph),
                "api_surface" => tool_api_surface(args, graph),
                "deps" => tool_deps(args, graph, project_root),
                _ => CallToolResult::error(format!(
                    "Unknown analysis: '{analysis}'. Use: dead_code, hotspots, architecture, patterns, api_surface, deps"
                )),
            }
        }
        "graphy_trace" => {
            let mode = args["mode"].as_str().unwrap_or("impact");
            match mode {
                "impact" => {
                    let r = tool_impact(args, graph);
                    if let Some(sym) = args["symbol"].as_str() {
                        for node in graph.find_by_name(sym) {
                            session.record(node.id, &node.name);
                        }
                    }
                    r
                }
                "taint" => tool_taint(args, graph),
                "dataflow" => tool_dataflow(args, graph),
                "tests" => tool_suggest_tests(args, graph),
                _ => CallToolResult::error(format!(
                    "Unknown trace mode: '{mode}'. Use: impact, taint, dataflow, tests"
                )),
            }
        }
        _ => CallToolResult::error(format!("Unknown tool: {name}")),
    };

    // Append graph confidence footer + session hints to successful responses
    if result.is_error.is_none() {
        append_confidence_footer(result, graph, session)
    } else {
        result
    }
}

/// Append graph confidence summary + session hints to a tool result.
fn append_confidence_footer(
    mut result: CallToolResult,
    graph: &CodeGraph,
    session: &SessionContext,
) -> CallToolResult {
    let footer = graph_confidence_summary(graph);
    let mut extra = format!("\n\n---\n{footer}");

    // Add session context hints
    let explored = session.explored_count();
    if explored > 0 {
        let neighbors = session.unexplored_neighbors(graph);
        if !neighbors.is_empty() {
            extra.push_str(&format!(
                "\nSession: {} symbols explored | Related unexplored: {}",
                explored,
                neighbors
                    .iter()
                    .map(|(name, reason)| format!("`{}` ({})", name, reason))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(block) = result.content.last_mut() {
        block.text.push_str(&extra);
    }
    result
}

/// Compute a one-line graph confidence summary.
fn graph_confidence_summary(graph: &CodeGraph) -> String {
    let mut total_callables = 0usize;
    let mut with_callers = 0usize;
    let mut with_callees = 0usize;
    let mut total_imports = 0usize;
    let mut resolved_imports = 0usize;
    let mut taint_count = 0usize;

    for n in graph.all_nodes() {
        if n.kind.is_callable() {
            total_callables += 1;
            if !graph.callers(n.id).is_empty() {
                with_callers += 1;
            }
            if !graph.callees(n.id).is_empty() {
                with_callees += 1;
            }
        }
        if n.kind == NodeKind::Import {
            total_imports += 1;
            if !graph.outgoing(n.id, EdgeKind::ImportsFrom).is_empty() {
                resolved_imports += 1;
            }
        }
        if !graph.outgoing(n.id, EdgeKind::TaintedBy).is_empty() {
            taint_count += 1;
        }
    }

    let call_pct = if total_callables > 0 {
        ((with_callers + with_callees) as f64 / (total_callables * 2) as f64 * 100.0) as u32
    } else {
        0
    };
    let import_pct = if total_imports > 0 {
        (resolved_imports as f64 / total_imports as f64 * 100.0) as u32
    } else {
        100
    };

    format!(
        "Graph: {} nodes, {} edges | Call coverage: {}% | Imports resolved: {}% ({}/{}){}",
        graph.node_count(),
        graph.edge_count(),
        call_pct,
        import_pct,
        resolved_imports,
        total_imports,
        if taint_count > 0 {
            format!(" | {} taint paths", taint_count)
        } else {
            String::new()
        }
    )
}

// ── Query Handlers ──────────────────────────────────────────

fn tool_search(args: &Value, graph: &CodeGraph, search: Option<&SearchIndex>) -> CallToolResult {
    let query = args["query"].as_str().unwrap_or("");
    if query.is_empty() {
        return CallToolResult::error("Missing required parameter: 'query'".to_string());
    }
    let max = (args["max_results"].as_u64().unwrap_or(20) as usize).min(100);
    let kind_filter = args["kind"].as_str();

    if let Some(idx) = search {
        let results = if let Some(kind) = kind_filter {
            idx.search_by_kind(query, kind, max)
        } else {
            idx.search(query, max)
        };

        match results {
            Ok(results) => {
                let text = results
                    .iter()
                    .map(|r| {
                        format!(
                            "- {} `{}` at {}:{}{}\n  Score: {:.2}",
                            r.kind,
                            r.name,
                            r.file_path,
                            r.start_line,
                            r.signature
                                .as_ref()
                                .map(|s| format!("\n  Signature: {s}"))
                                .unwrap_or_default(),
                            r.score
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");

                let header = format!("Found {} results for '{query}':\n\n", results.len());
                CallToolResult::text(header + &text)
            }
            Err(e) => CallToolResult::error(format!("Search failed: {e}")),
        }
    } else {
        // Fallback to graph-based name search
        let mut matches: Vec<&GirNode> = graph
            .all_nodes()
            .filter(|n| n.name.to_lowercase().contains(&query.to_lowercase()))
            .filter(|n| {
                kind_filter
                    .map(|k| format!("{:?}", n.kind) == k)
                    .unwrap_or(true)
            })
            .collect();
        matches.sort_by_key(|n| n.name.len());
        matches.truncate(max);

        let text = format_nodes(&matches);
        CallToolResult::text(format!(
            "Found {} results for '{query}':\n\n{text}",
            matches.len()
        ))
    }
}

fn tool_context(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let symbol = args["symbol"].as_str().unwrap_or("");
    if symbol.is_empty() {
        return CallToolResult::error("Missing required parameter: 'query'".to_string());
    }
    let detail = args["detail_level"].as_str().unwrap_or("normal");

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        return CallToolResult::text(format!(
            "Symbol '{symbol}' not found. Use graphy_query with mode=search to find the exact name."
        ));
    }

    let mut out = String::new();
    for node in &nodes {
        out.push_str(&format!("## {} ({:?})\n", node.name, node.kind));
        out.push_str(&format!(
            "- File: `{}:{}`\n",
            node.file_path.display(),
            node.span.start_line
        ));
        out.push_str(&format!("- Visibility: {:?}\n", node.visibility));

        if let Some(sig) = &node.signature {
            out.push_str(&format!("- Signature: `{sig}`\n"));
        }
        if let Some(doc) = &node.doc {
            out.push_str(&format!("- Doc: {doc}\n"));
        }
        if let Some(cx) = &node.complexity {
            out.push_str(&format!(
                "- Complexity: cyclomatic={}, cognitive={}, loc={}\n",
                cx.cyclomatic, cx.cognitive, cx.loc
            ));
        }

        // Full source code of the queried symbol
        if let Some(source) =
            read_source_span(&node.file_path, node.span.start_line, node.span.end_line)
        {
            out.push_str(&format!("\n### Source\n```\n{}\n```\n", source));
        }

        // Callers with full source
        let callers = graph.callers(node.id);
        if !callers.is_empty() {
            out.push_str(&format!("\n### Callers ({})\n", callers.len()));
            let limit = if detail == "verbose" { callers.len() } else { 10 };
            for c in callers.iter().take(limit) {
                out.push_str(&format!(
                    "- `{}` at {}:{}\n",
                    c.name,
                    c.file_path.display(),
                    c.span.start_line
                ));
                if let Some(src) =
                    read_source_span(&c.file_path, c.span.start_line, c.span.end_line)
                {
                    out.push_str(&format!("  ```\n{}\n  ```\n", indent(&src, "  ")));
                }
            }
            if callers.len() > limit {
                out.push_str(&format!("... and {} more\n", callers.len() - limit));
            }
        }

        // Callees with full source
        let callees = graph.callees(node.id);
        if !callees.is_empty() {
            out.push_str(&format!("\n### Callees ({})\n", callees.len()));
            let limit = if detail == "verbose" { 50 } else { 10 };
            for c in callees.iter().take(limit) {
                out.push_str(&format!(
                    "- `{}` at {}:{}\n",
                    c.name,
                    c.file_path.display(),
                    c.span.start_line
                ));
                if let Some(src) =
                    read_source_span(&c.file_path, c.span.start_line, c.span.end_line)
                {
                    out.push_str(&format!("  ```\n{}\n  ```\n", indent(&src, "  ")));
                }
            }
            if callees.len() > limit {
                out.push_str(&format!("... and {} more\n", callees.len() - limit));
            }
        }

        // Children
        let children = graph.children(node.id);
        if !children.is_empty() && detail != "summary" {
            let child_limit = 50;
            out.push_str(&format!("\n### Contains ({})\n", children.len()));
            for c in children.iter().take(child_limit) {
                out.push_str(&format!("- {:?} `{}`\n", c.kind, c.name));
            }
            if children.len() > child_limit {
                out.push_str(&format!("... and {} more\n", children.len() - child_limit));
            }
        }

        out.push('\n');
    }

    CallToolResult::text(out)
}

fn tool_explain(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let symbol = args["symbol"].as_str().unwrap_or("");
    if symbol.is_empty() {
        return CallToolResult::error("Missing required parameter: 'query'".to_string());
    }
    let include_source = args["include_source"].as_bool().unwrap_or(true);

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        return CallToolResult::text(format!(
            "Symbol '{symbol}' not found. Use graphy_query with mode=search to find the exact name."
        ));
    }

    let mut out = String::new();
    for node in &nodes {
        out.push_str(&format!("# {} ({:?})\n\n", node.name, node.kind));
        out.push_str(&format!(
            "**File:** `{}:{}`\n",
            node.file_path.display(),
            node.span.start_line
        ));
        out.push_str(&format!("**Visibility:** {:?}\n", node.visibility));
        out.push_str(&format!("**Language:** {:?}\n", node.language));

        if let Some(sig) = &node.signature {
            out.push_str(&format!("**Signature:** `{sig}`\n"));
        }
        if let Some(doc) = &node.doc {
            out.push_str(&format!("\n**Documentation:** {doc}\n"));
        }

        // Source code
        if include_source {
            if let Some(source) =
                read_source_span(&node.file_path, node.span.start_line, node.span.end_line)
            {
                out.push_str(&format!("\n## Source Code\n```\n{}\n```\n", source));
            }
        }

        // Callers with source
        let callers = graph.callers(node.id);
        if !callers.is_empty() {
            out.push_str(&format!("\n## Callers ({}):\n", callers.len()));
            for (i, c) in callers.iter().take(20).enumerate() {
                out.push_str(&format!(
                    "- `{}` ({:?}) at {}:{}\n",
                    c.name,
                    c.kind,
                    c.file_path.display(),
                    c.span.start_line
                ));
                if i < 10 {
                    if let Some(src) = read_source_context(&c.file_path, c.span.start_line, 1) {
                        out.push_str(&format!("  ```\n{}\n  ```\n", indent(&src, "  ")));
                    }
                }
            }
            if callers.len() > 20 {
                out.push_str(&format!("... and {} more\n", callers.len() - 20));
            }
        }

        // Callees
        let callees = graph.callees(node.id);
        if !callees.is_empty() {
            out.push_str(&format!("\n## Callees ({}):\n", callees.len()));
            for c in callees.iter().take(20) {
                out.push_str(&format!(
                    "- `{}` ({:?}) at {}:{}\n",
                    c.name,
                    c.kind,
                    c.file_path.display(),
                    c.span.start_line
                ));
            }
            if callees.len() > 20 {
                out.push_str(&format!("... and {} more\n", callees.len() - 20));
            }
        }

        // Type hierarchy
        let inherits = graph.outgoing(node.id, EdgeKind::Inherits);
        let implements = graph.outgoing(node.id, EdgeKind::Implements);
        if !inherits.is_empty() || !implements.is_empty() {
            out.push_str("\n## Type Hierarchy\n");
            for parent in &inherits {
                out.push_str(&format!("- Inherits from `{}`\n", parent.name));
            }
            for iface in &implements {
                out.push_str(&format!("- Implements `{}`\n", iface.name));
            }
        }

        // Complexity
        if let Some(cx) = &node.complexity {
            out.push_str(&format!(
                "\n## Complexity\n- Cyclomatic: {}\n- Cognitive: {}\n- LOC: {}\n- Parameters: {}\n- Max nesting: {}\n",
                cx.cyclomatic, cx.cognitive, cx.loc, cx.parameter_count, cx.max_nesting_depth
            ));
        }

        out.push_str(&format!(
            "\n## Liveness: {:.0}%\n",
            node.confidence * 100.0
        ));
        out.push('\n');
    }

    CallToolResult::text(out)
}

fn tool_changes(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let file_path = args["file_path"]
        .as_str()
        .or_else(|| args["query"].as_str())
        .unwrap_or("");
    if file_path.is_empty() {
        return CallToolResult::error("Missing required parameter: 'query' (file path)".to_string());
    }
    let path = std::path::Path::new(file_path);

    let nodes = graph.find_by_file(path);
    if nodes.is_empty() {
        return CallToolResult::text(format!(
            "No symbols found in '{file_path}'. Make sure the file has been indexed."
        ));
    }

    let mut out = format!("## Symbols in `{file_path}` ({} total)\n\n", nodes.len());

    let callables: Vec<_> = nodes.iter().filter(|n| n.kind.is_callable()).collect();
    for node in &callables {
        let callers = graph.callers(node.id);
        out.push_str(&format!("### `{}` ({:?})\n", node.name, node.kind));

        // Show signature
        if let Some(sig) = &node.signature {
            out.push_str(&format!("  Signature: `{sig}`\n"));
        }

        if callers.is_empty() {
            out.push_str("  No external callers\n\n");
        } else {
            out.push_str(&format!("  {} callers:\n", callers.len()));
            for (i, c) in callers.iter().take(5).enumerate() {
                out.push_str(&format!(
                    "  - `{}` at {}:{}\n",
                    c.name,
                    c.file_path.display(),
                    c.span.start_line
                ));
                if i < 3 {
                    if let Some(src) = read_source_context(&c.file_path, c.span.start_line, 0) {
                        out.push_str(&format!("    ```\n{}\n    ```\n", indent(&src, "    ")));
                    }
                }
            }
            if callers.len() > 5 {
                out.push_str(&format!("  ... and {} more\n", callers.len() - 5));
            }
            out.push('\n');
        }
    }

    CallToolResult::text(out)
}

// ── Analyze Handlers ────────────────────────────────────────

fn tool_dead_code(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let max = args["max_results"].as_u64().unwrap_or(20) as usize;

    let callable = [NodeKind::Function, NodeKind::Method];
    let mut dead: Vec<&GirNode> = graph
        .all_nodes()
        .filter(|n| callable.contains(&n.kind))
        .filter(|n| !graph.is_phantom(n.id))
        .filter(|n| n.confidence < 0.5)
        .collect();

    dead.sort_by(|a, b| a.confidence.total_cmp(&b.confidence));
    dead.truncate(max);

    let mut out = format!("## Dead Code Report ({} candidates)\n\n", dead.len());
    for node in &dead {
        let cov_str = node
            .coverage
            .map(|c| format!(" | coverage: {:.0}%", c * 100.0))
            .unwrap_or_default();
        out.push_str(&format!(
            "### `{}` ({:?}) — liveness: {:.0}%{} — {}:{}\n",
            node.name,
            node.kind,
            node.confidence * 100.0,
            cov_str,
            node.file_path.display(),
            node.span.start_line
        ));
        // Show first 3 lines of the function (signature + start of body)
        if let Some(src) = read_source_preview(&node.file_path, node.span.start_line, 3) {
            out.push_str(&format!("```\n{}\n```\n\n", src));
        }
    }

    CallToolResult::text(out)
}

fn tool_hotspots(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let max = args["max_results"].as_u64().unwrap_or(20) as usize;

    let mut hotspots: Vec<(&GirNode, f64, usize)> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable())
        .filter_map(|n| {
            let cx = n.complexity.as_ref()?;
            let caller_count = graph.callers(n.id).len();
            let complexity = cx.cyclomatic as f64;
            let risk = complexity * (1.0 + caller_count as f64 * 0.5);
            Some((n, risk, caller_count))
        })
        .collect();

    hotspots.sort_by(|a, b| b.1.total_cmp(&a.1));
    hotspots.truncate(max);

    let mut out = format!("## Hotspots ({} ranked by risk)\n\n", hotspots.len());
    out.push_str("| Risk | Symbol | Complexity | Callers | Location |\n");
    out.push_str("|------|--------|-----------|---------|----------|\n");

    for (node, risk, caller_count) in &hotspots {
        let Some(cx) = node.complexity.as_ref() else {
            continue;
        };
        out.push_str(&format!(
            "| {:.1} | `{}` | cyc={} cog={} | {} | {}:{} |\n",
            risk,
            node.name,
            cx.cyclomatic,
            cx.cognitive,
            caller_count,
            node.file_path.display(),
            node.span.start_line
        ));
    }

    if hotspots.is_empty() {
        out.push_str("No complexity metrics available. Run `graphy analyze` first.\n");
    }

    CallToolResult::text(out)
}

fn tool_architecture(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let detail = args["detail_level"].as_str().unwrap_or("summary");

    let files = graph.find_by_kind(NodeKind::File);
    let classes = graph.find_by_kind(NodeKind::Class);
    let functions = graph.find_by_kind(NodeKind::Function);
    let methods = graph.find_by_kind(NodeKind::Method);
    let imports = graph.find_by_kind(NodeKind::Import);

    let mut out = format!(
        "## Architecture Overview\n\n\
         - **Files**: {}\n\
         - **Classes**: {}\n\
         - **Functions**: {}\n\
         - **Methods**: {}\n\
         - **Imports**: {}\n\
         - **Total nodes**: {}\n\
         - **Total edges**: {}\n\n",
        files.len(),
        classes.len(),
        functions.len(),
        methods.len(),
        imports.len(),
        graph.node_count(),
        graph.edge_count(),
    );

    // Show files with most symbols
    let mut file_counts: HashMap<&std::path::Path, usize> = HashMap::new();
    for node in graph.all_nodes() {
        *file_counts.entry(&node.file_path).or_default() += 1;
    }
    let mut sorted: Vec<_> = file_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    out.push_str("### Largest Files\n");
    let file_limit = if detail == "verbose" { 20 } else { 10 };
    for (path, count) in sorted.iter().take(file_limit) {
        out.push_str(&format!("- `{}` — {} symbols\n", path.display(), count));
    }

    if detail != "summary" {
        // Show language breakdown
        let mut lang_counts: HashMap<String, usize> = HashMap::new();
        for node in graph.all_nodes() {
            *lang_counts
                .entry(format!("{:?}", node.language))
                .or_default() += 1;
        }
        let mut lang_sorted: Vec<_> = lang_counts.into_iter().collect();
        lang_sorted.sort_by(|a, b| b.1.cmp(&a.1));

        out.push_str("\n### Languages\n");
        for (lang, count) in &lang_sorted {
            out.push_str(&format!("- {lang}: {count} symbols\n"));
        }

        // Show entry points (public functions with no callers)
        let entries: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && n.visibility == Visibility::Public)
            .filter(|n| graph.callers(n.id).is_empty())
            .take(15)
            .collect();
        if !entries.is_empty() {
            out.push_str(&format!("\n### Entry Points ({} public, uncalled)\n", entries.len()));
            for e in &entries {
                out.push_str(&format!(
                    "- `{}` ({:?}) at {}:{}\n",
                    e.name,
                    e.kind,
                    e.file_path.display(),
                    e.span.start_line
                ));
            }
        }
    }

    CallToolResult::text(out)
}

fn tool_patterns(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let max = args["max_results"].as_u64().unwrap_or(20) as usize;
    let mut findings = Vec::new();

    for node in graph.all_nodes() {
        // God classes (>15 methods)
        if node.kind == NodeKind::Class {
            let method_count = graph.children(node.id)
                .iter()
                .filter(|c| c.kind == NodeKind::Method || c.kind == NodeKind::Constructor)
                .count();
            if method_count > 15 {
                findings.push(format!(
                    "- **God Class**: `{}` has {} methods — {}:{}",
                    node.name, method_count, node.file_path.display(), node.span.start_line
                ));
            }
        }

        if node.kind.is_callable() {
            let children = graph.children(node.id);

            // Long parameter lists (>5 params)
            let param_count = children.iter().filter(|c| c.kind == NodeKind::Parameter).count();
            if param_count > 5 {
                findings.push(format!(
                    "- **Long Parameter List**: `{}` has {} parameters — {}:{}",
                    node.name, param_count, node.file_path.display(), node.span.start_line
                ));
            }

            // High complexity
            if let Some(cx) = &node.complexity {
                if cx.cyclomatic > 15 {
                    findings.push(format!(
                        "- **High Complexity**: `{}` cyclomatic={} — {}:{}",
                        node.name, cx.cyclomatic, node.file_path.display(), node.span.start_line
                    ));
                }
            }
        }
    }

    findings.truncate(max);

    let mut out = format!("## Pattern Analysis ({} findings)\n\n", findings.len());
    for f in &findings {
        out.push_str(f);
        out.push('\n');
    }

    if findings.is_empty() {
        out.push_str("No anti-patterns detected.\n");
    }

    CallToolResult::text(out)
}

fn tool_api_surface(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let file_filter = args["file_path"].as_str();

    let mut public = Vec::new();
    let mut internal = Vec::new();

    for node in graph.all_nodes() {
        if !node.kind.is_callable() && !node.kind.is_type_def() {
            continue;
        }
        if let Some(fp) = file_filter {
            if node.file_path.to_string_lossy() != fp {
                continue;
            }
        }

        let callers = graph.callers(node.id);
        let external_callers = callers
            .iter()
            .filter(|c| c.file_path != node.file_path)
            .count();

        let effective = if node.visibility == Visibility::Public && external_callers == 0 {
            "effectively_internal"
        } else if node.visibility == Visibility::Public {
            "public_api"
        } else {
            "internal"
        };

        match effective {
            "public_api" => public.push(node),
            _ => internal.push(node),
        }
    }

    let mut out = format!(
        "## API Surface\n\n### Public API ({} symbols)\n",
        public.len()
    );
    for n in &public {
        out.push_str(&format!(
            "- {:?} `{}` at {}:{}\n",
            n.kind,
            n.name,
            n.file_path.display(),
            n.span.start_line
        ));
    }

    out.push_str(&format!(
        "\n### Internal ({} symbols)\n",
        internal.len()
    ));
    for n in internal.iter().take(20) {
        out.push_str(&format!(
            "- {:?} `{}` ({:?})\n",
            n.kind, n.name, n.visibility
        ));
    }
    if internal.len() > 20 {
        out.push_str(&format!("... and {} more\n", internal.len() - 20));
    }

    CallToolResult::text(out)
}

fn tool_deps(args: &Value, graph: &CodeGraph, project_root: &Path) -> CallToolResult {
    let check_vulns = args["check_vulns"].as_bool().unwrap_or(false);

    let root = project_root.to_path_buf();
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| {
            handle.block_on(graphy_deps::analyze_dependencies(
                &root,
                Some(graph),
                check_vulns,
            ))
        }),
        Err(_) => {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    return CallToolResult::error(format!("Failed to create runtime: {e}"));
                }
            };
            rt.block_on(graphy_deps::analyze_dependencies(
                &root,
                Some(graph),
                check_vulns,
            ))
        }
    };

    match result {
        Ok(analysis) => CallToolResult::text(graphy_deps::format_deps_text(&analysis)),
        Err(e) => CallToolResult::error(format!("Dependency analysis failed: {e}")),
    }
}

// ── Trace Handlers ──────────────────────────────────────────

fn tool_impact(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let symbol = args["symbol"].as_str().unwrap_or("");
    if symbol.is_empty() {
        return CallToolResult::error(
            "Missing required parameter: 'symbol'".to_string(),
        );
    }
    let max_depth = (args["max_depth"].as_u64().unwrap_or(3) as usize).min(10);

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        return CallToolResult::text(format!("Symbol '{symbol}' not found."));
    }

    const MAX_VISITED: usize = 10_000;

    let mut out = String::new();
    for node in &nodes {
        out.push_str(&format!(
            "## Impact analysis for `{}` ({:?})\n\n",
            node.name, node.kind
        ));

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut by_depth: HashMap<usize, Vec<&GirNode>> = HashMap::new();

        visited.insert(node.id);
        queue.push_back((node.id, 0usize));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth || visited.len() >= MAX_VISITED {
                continue;
            }
            for caller in graph.callers(current_id) {
                if visited.insert(caller.id) {
                    by_depth.entry(depth + 1).or_default().push(caller);
                    queue.push_back((caller.id, depth + 1));
                }
            }
        }

        let total = visited.len() - 1;
        out.push_str(&format!("**Total affected symbols: {total}**\n\n"));

        for depth in 1..=max_depth {
            if let Some(nodes_at_depth) = by_depth.get(&depth) {
                out.push_str(&format!(
                    "### Depth {depth} ({} symbols)\n",
                    nodes_at_depth.len()
                ));
                for n in nodes_at_depth.iter() {
                    out.push_str(&format!(
                        "- `{}` ({:?}) at {}:{}\n",
                        n.name,
                        n.kind,
                        n.file_path.display(),
                        n.span.start_line
                    ));
                    if let Some(src) =
                        read_source_span(&n.file_path, n.span.start_line, n.span.end_line)
                    {
                        out.push_str(&format!("  ```\n{}\n  ```\n", indent(&src, "  ")));
                    }
                }
                out.push('\n');
            }
        }
    }

    CallToolResult::text(out)
}

fn tool_taint(_args: &Value, graph: &CodeGraph) -> CallToolResult {
    let taint_edges: Vec<_> = graph
        .all_nodes()
        .filter(|n| {
            graph
                .outgoing(n.id, EdgeKind::TaintedBy)
                .first()
                .is_some()
        })
        .collect();

    if taint_edges.is_empty() {
        return CallToolResult::text(
            "No taint paths detected. This may mean the code is safe, or taint analysis needs more data.\n\
             Re-run `graphy analyze` to ensure taint analysis is complete."
                .into(),
        );
    }

    let taint_limit = 100;
    let mut out = format!(
        "## Taint Analysis ({} tainted symbols)\n\n",
        taint_edges.len()
    );
    for node in taint_edges.iter().take(taint_limit) {
        let sources = graph.outgoing(node.id, EdgeKind::TaintedBy);
        out.push_str(&format!(
            "### `{}` at {}:{}\n",
            node.name,
            node.file_path.display(),
            node.span.start_line
        ));
        for src in sources.iter().take(10) {
            out.push_str(&format!("  Tainted by: `{}`\n", src.name));
        }
        // Show source around the tainted symbol
        if let Some(src) = read_source_context(&node.file_path, node.span.start_line, 2) {
            out.push_str(&format!("```\n{}\n```\n\n", src));
        }
    }
    if taint_edges.len() > taint_limit {
        out.push_str(&format!(
            "\n... and {} more tainted symbols\n",
            taint_edges.len() - taint_limit
        ));
    }
    CallToolResult::text(out)
}

fn tool_dataflow(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let symbol = args["symbol"].as_str().unwrap_or("");
    if symbol.is_empty() {
        return CallToolResult::error(
            "Missing required parameter: 'symbol'".to_string(),
        );
    }
    let nodes = graph.find_by_name(symbol);

    if nodes.is_empty() {
        return CallToolResult::text(format!("Symbol '{symbol}' not found."));
    }

    let mut out = String::new();
    for node in &nodes {
        out.push_str(&format!("## Data flow for `{}`\n\n", node.name));

        let outgoing = graph.outgoing(node.id, EdgeKind::DataFlowsTo);
        let incoming = graph.incoming(node.id, EdgeKind::DataFlowsTo);

        if !incoming.is_empty() {
            out.push_str("### Data flows IN from:\n");
            for n in &incoming {
                out.push_str(&format!("- `{}` ({:?})\n", n.name, n.kind));
            }
        }

        if !outgoing.is_empty() {
            out.push_str("### Data flows OUT to:\n");
            for n in &outgoing {
                out.push_str(&format!("- `{}` ({:?})\n", n.name, n.kind));
            }
        }

        if incoming.is_empty() && outgoing.is_empty() {
            out.push_str("No data flow edges detected for this symbol.\n");
        }
        out.push('\n');
    }

    CallToolResult::text(out)
}

fn tool_suggest_tests(args: &Value, graph: &CodeGraph) -> CallToolResult {
    let symbol = args["symbol"].as_str().unwrap_or("");
    if symbol.is_empty() {
        return CallToolResult::error(
            "Missing required parameter: 'symbol'".to_string(),
        );
    }
    let max_depth = (args["max_depth"].as_u64().unwrap_or(5) as usize).min(20);

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        return CallToolResult::text(format!("Symbol '{symbol}' not found."));
    }

    let mut out = String::new();
    for node in &nodes {
        out.push_str(&format!(
            "## Tests for `{}` ({:?})\n\n",
            node.name, node.kind
        ));

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut test_results: Vec<(usize, &GirNode)> = Vec::new();

        visited.insert(node.id);
        queue.push_back((node.id, 0usize));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for caller in graph.callers(current_id) {
                if visited.insert(caller.id) {
                    if is_test_function(caller, graph) {
                        test_results.push((depth + 1, caller));
                    }
                    queue.push_back((caller.id, depth + 1));
                }
            }
        }

        test_results.sort_by_key(|(depth, _)| *depth);

        if test_results.is_empty() {
            out.push_str("No test functions found in the call graph.\n\n");
            out.push_str("**Suggestion:** Write tests that call this function directly.\n");
        } else {
            out.push_str(&format!("Found {} test(s):\n\n", test_results.len()));
            for (depth, test_node) in &test_results {
                out.push_str(&format!(
                    "- `{}` (depth {}) — {}:{}\n",
                    test_node.name,
                    depth,
                    test_node.file_path.display(),
                    test_node.span.start_line
                ));
            }
        }
        out.push('\n');
    }

    CallToolResult::text(out)
}

// ── Helpers ─────────────────────────────────────────────────

/// Read lines from a source file, returning None for files >10MB or on error.
fn read_file_lines(path: &Path) -> Option<Vec<String>> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.len() > 10_000_000 {
        return None;
    }
    Some(content.lines().map(String::from).collect())
}

/// Read source code for a span from disk.
fn read_source_span(path: &Path, start_line: u32, end_line: u32) -> Option<String> {
    let lines = read_file_lines(path)?;
    let start = start_line as usize;
    let end = (end_line as usize).min(lines.len());
    if start >= lines.len() {
        return None;
    }
    Some(lines[start..end].join("\n"))
}

/// Read a few lines of source around a given line number.
/// `context` is the number of lines before/after the center line.
fn read_source_context(path: &Path, center_line: u32, context: u32) -> Option<String> {
    let lines = read_file_lines(path)?;
    let center = center_line as usize;
    if center >= lines.len() {
        return None;
    }
    let start = center.saturating_sub(context as usize);
    let end = (center + context as usize + 1).min(lines.len());
    Some(
        lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:>4} | {}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Read the first N lines starting from a given line.
fn read_source_preview(path: &Path, start_line: u32, num_lines: u32) -> Option<String> {
    let lines = read_file_lines(path)?;
    let start = start_line as usize;
    if start >= lines.len() {
        return None;
    }
    let end = (start + num_lines as usize).min(lines.len());
    Some(
        lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:>4} | {}", start + i + 1, l))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Indent every line of text with a prefix.
fn indent(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Check if a node is a test function.
fn is_test_function(node: &GirNode, graph: &CodeGraph) -> bool {
    let name = &node.name;

    if name.starts_with("test_") || name.starts_with("Test") || name.starts_with("it_") {
        return true;
    }

    let decorators = graph.outgoing(node.id, EdgeKind::AnnotatedWith);
    if decorators.iter().any(|d| {
        let n = d.name.to_lowercase();
        n == "test" || n == "pytest.mark" || n == "it" || n == "describe"
    }) {
        return true;
    }

    let file_str = node.file_path.to_string_lossy();
    if file_str.contains("_test.rs")
        || file_str.contains("test_")
        || file_str.contains(".test.ts")
        || file_str.contains(".test.js")
        || file_str.contains(".spec.ts")
        || file_str.contains(".spec.js")
        || file_str.contains("/tests/")
        || file_str.contains("/__tests__/")
        || file_str.contains("/test/")
    {
        return true;
    }

    false
}

fn format_nodes(nodes: &[&GirNode]) -> String {
    nodes
        .iter()
        .map(|n| {
            format!(
                "- {:?} `{}` at {}:{}{}",
                n.kind,
                n.name,
                n.file_path.display(),
                n.span.start_line,
                n.signature
                    .as_ref()
                    .map(|s| format!("\n  Signature: {s}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirEdge, GirNode, Language, Span};
    use std::path::PathBuf;

    fn make_fn(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    fn make_class(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Class,
            PathBuf::from(file),
            Span::new(line, 0, line + 20, 0),
            Language::Python,
        )
    }

    fn make_import(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Import,
            PathBuf::from(file),
            Span::new(line, 0, line, 30),
            Language::Python,
        )
    }

    fn make_param(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Parameter,
            PathBuf::from(file),
            Span::new(line, 0, line, 10),
            Language::Python,
        )
    }

    // ── SessionContext tests ────────────────────────────────

    #[test]
    fn session_context_new_empty() {
        let ctx = SessionContext::new();
        assert_eq!(ctx.explored_count(), 0);
    }

    #[test]
    fn session_context_record_and_count() {
        let ctx = SessionContext::new();
        let node = make_fn("foo", "a.py", 1);
        ctx.record(node.id, "foo");
        assert_eq!(ctx.explored_count(), 1);
        // Recording the same ID again doesn't increase count
        ctx.record(node.id, "foo");
        assert_eq!(ctx.explored_count(), 1);
    }

    #[test]
    fn session_context_record_nodes() {
        let ctx = SessionContext::new();
        let n1 = make_fn("foo", "a.py", 1);
        let n2 = make_fn("bar", "b.py", 1);
        ctx.record_nodes(&[&n1, &n2]);
        assert_eq!(ctx.explored_count(), 2);
    }

    #[test]
    fn session_context_unexplored_neighbors_empty() {
        let ctx = SessionContext::new();
        let graph = CodeGraph::new();
        let neighbors = ctx.unexplored_neighbors(&graph);
        assert!(neighbors.is_empty());
    }

    #[test]
    fn session_context_unexplored_neighbors_with_callers() {
        let ctx = SessionContext::new();
        let mut graph = CodeGraph::new();

        let callee = make_fn("callee", "a.py", 1);
        let caller = make_fn("caller", "a.py", 10);
        let callee_id = callee.id;
        let caller_id = caller.id;

        graph.add_node(callee);
        graph.add_node(caller);
        graph.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));

        // Mark callee as explored
        ctx.record(callee_id, "callee");

        let neighbors = ctx.unexplored_neighbors(&graph);
        assert!(!neighbors.is_empty());
        assert!(neighbors.iter().any(|(name, _)| name == "caller"));
    }

    // ── tool_definitions tests ─────────────────────────────

    #[test]
    fn tool_definitions_returns_three_tools() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 3);
    }

    #[test]
    fn tool_definitions_correct_names() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"graphy_query"));
        assert!(names.contains(&"graphy_analyze"));
        assert!(names.contains(&"graphy_trace"));
    }

    #[test]
    fn tool_definitions_have_schemas() {
        let defs = tool_definitions();
        for def in &defs {
            assert!(!def.description.is_empty());
            assert!(def.input_schema.is_object());
        }
    }

    // ── handle_tool dispatch tests ─────────────────────────

    #[test]
    fn handle_tool_unknown_tool() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "nonexistent",
            &json!({}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("Unknown tool"));
    }

    #[test]
    fn handle_tool_query_missing_query() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"mode": "search"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("query"));
    }

    #[test]
    fn handle_tool_query_empty_query() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": ""}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
    }

    #[test]
    fn handle_tool_query_search_no_results() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "nonexistent", "mode": "search"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        // Not an error, just 0 results
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("0 results"));
    }

    #[test]
    fn handle_tool_query_search_finds_symbol() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("process_data", "app.py", 1));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "process_data", "mode": "search"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("process_data"));
    }

    #[test]
    fn handle_tool_query_search_kind_filter() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("handle", "app.py", 1));
        graph.add_node(make_class("handle", "app.py", 20));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "handle", "mode": "search", "kind": "Function"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("Function"));
        // Should not include the Class
        assert!(!text.contains("Class `handle`"));
    }

    #[test]
    fn handle_tool_query_context_not_found() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "ghost", "mode": "context"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn handle_tool_query_context_found() {
        let mut graph = CodeGraph::new();
        let f = make_fn("my_func", "app.py", 1);
        graph.add_node(f);

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "my_func", "mode": "context"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("my_func"));
        // Session should track the explored symbol
        assert_eq!(session.explored_count(), 1);
    }

    #[test]
    fn handle_tool_query_explain_not_found() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "ghost", "mode": "explain"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn handle_tool_query_file_mode_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"query": "app.py", "mode": "file"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("No symbols found"));
    }

    #[test]
    fn handle_tool_query_batch_queries() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("alpha", "a.py", 1));
        graph.add_node(make_fn("beta", "b.py", 1));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"queries": ["alpha", "beta"], "mode": "search"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("alpha"));
        assert!(text.contains("beta"));
    }

    #[test]
    fn handle_tool_query_batch_empty_array() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_query",
            &json!({"queries": [], "mode": "search"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("empty"));
    }

    // ── Analyze tool tests ─────────────────────────────────

    #[test]
    fn handle_tool_analyze_unknown_analysis() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "nonexistent"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("Unknown analysis"));
    }

    #[test]
    fn handle_tool_analyze_dead_code_empty_graph() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "dead_code"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Dead Code Report"));
    }

    fn make_file_node(file: &str) -> GirNode {
        GirNode::new(
            file.to_string(),
            NodeKind::File,
            PathBuf::from(file),
            Span::new(0, 0, 100, 0),
            Language::Python,
        )
    }

    /// Add a function with a parent File node so it's not considered phantom.
    fn add_fn_with_parent(graph: &mut CodeGraph, name: &str, file: &str, line: u32, confidence: f32) {
        // Ensure file node exists
        let file_node = make_file_node(file);
        let file_id = file_node.id;
        if graph.get_node(file_id).is_none() {
            graph.add_node(file_node);
        }
        let mut func = make_fn(name, file, line);
        func.confidence = confidence;
        let func_id = func.id;
        graph.add_node(func);
        graph.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));
    }

    #[test]
    fn handle_tool_analyze_dead_code_finds_dead() {
        let mut graph = CodeGraph::new();
        add_fn_with_parent(&mut graph, "unused_func", "app.py", 1, 0.1);

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "dead_code"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("unused_func"));
    }

    #[test]
    fn handle_tool_analyze_dead_code_excludes_alive() {
        let mut graph = CodeGraph::new();
        add_fn_with_parent(&mut graph, "alive_func", "app.py", 1, 0.9);

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "dead_code"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(!result.content[0].text.contains("alive_func"));
    }

    #[test]
    fn handle_tool_analyze_hotspots_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "hotspots"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Hotspots"));
    }

    #[test]
    fn handle_tool_analyze_architecture_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "architecture"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("Architecture Overview"));
    }

    #[test]
    fn handle_tool_analyze_architecture_with_nodes() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("main", "app.py", 1));
        graph.add_node(make_class("MyClass", "app.py", 10));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "architecture", "detail_level": "normal"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("Functions"));
        assert!(text.contains("Classes"));
    }

    #[test]
    fn handle_tool_analyze_patterns_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "patterns"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("No anti-patterns"));
    }

    #[test]
    fn handle_tool_analyze_patterns_detects_long_params() {
        let mut graph = CodeGraph::new();
        let func = make_fn("complex_func", "app.py", 1);
        let func_id = func.id;
        graph.add_node(func);

        // Add 6 parameters — triggers "Long Parameter List"
        for i in 0..6 {
            let p = make_param(&format!("param{i}"), "app.py", 1);
            let pid = p.id;
            graph.add_node(p);
            graph.add_edge(func_id, pid, GirEdge::new(EdgeKind::Contains));
        }

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "patterns"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.content[0].text.contains("Long Parameter List"));
    }

    #[test]
    fn handle_tool_analyze_api_surface_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "api_surface"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("API Surface"));
    }

    // ── Trace tool tests ───────────────────────────────────

    #[test]
    fn handle_tool_trace_unknown_mode() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "nonexistent", "symbol": "x"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("Unknown trace mode"));
    }

    #[test]
    fn handle_tool_trace_impact_missing_symbol() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "impact"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
        assert!(result.content[0].text.contains("symbol"));
    }

    #[test]
    fn handle_tool_trace_impact_not_found() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "impact", "symbol": "ghost"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn handle_tool_trace_impact_with_callers() {
        let mut graph = CodeGraph::new();
        let callee = make_fn("target", "a.py", 1);
        let caller = make_fn("caller", "a.py", 10);
        let callee_id = callee.id;
        let caller_id = caller.id;
        graph.add_node(callee);
        graph.add_node(caller);
        graph.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "impact", "symbol": "target"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("Impact analysis"));
        assert!(text.contains("caller"));
    }

    #[test]
    fn handle_tool_trace_taint_empty() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "taint"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("No taint paths"));
    }

    #[test]
    fn handle_tool_trace_taint_with_edges() {
        let mut graph = CodeGraph::new();
        let source = make_fn("user_input", "a.py", 1);
        let sink = make_fn("execute_sql", "a.py", 10);
        let source_id = source.id;
        let sink_id = sink.id;
        graph.add_node(source);
        graph.add_node(sink);
        graph.add_edge(sink_id, source_id, GirEdge::new(EdgeKind::TaintedBy));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "taint"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        let text = &result.content[0].text;
        assert!(text.contains("execute_sql"));
        assert!(text.contains("user_input"));
    }

    #[test]
    fn handle_tool_trace_dataflow_missing_symbol() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "dataflow"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
    }

    #[test]
    fn handle_tool_trace_dataflow_no_edges() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("lonely", "a.py", 1));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "dataflow", "symbol": "lonely"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("No data flow edges"));
    }

    #[test]
    fn handle_tool_trace_tests_missing_symbol() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "tests"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_some());
    }

    #[test]
    fn handle_tool_trace_tests_no_tests() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("untested_func", "a.py", 1));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "tests", "symbol": "untested_func"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("No test functions"));
    }

    #[test]
    fn handle_tool_trace_tests_finds_test() {
        let mut graph = CodeGraph::new();
        let target = make_fn("my_func", "src/lib.py", 1);
        let test = make_fn("test_my_func", "tests/test_lib.py", 1);
        let target_id = target.id;
        let test_id = test.id;
        graph.add_node(target);
        graph.add_node(test);
        graph.add_edge(test_id, target_id, GirEdge::new(EdgeKind::Calls));

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_trace",
            &json!({"mode": "tests", "symbol": "my_func"}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        assert!(result.content[0].text.contains("test_my_func"));
    }

    // ── Helper function tests ──────────────────────────────

    #[test]
    fn test_is_test_function_by_name() {
        let graph = CodeGraph::new();
        let t1 = make_fn("test_something", "test.py", 1);
        assert!(is_test_function(&t1, &graph));
        let t2 = make_fn("TestCase", "test.py", 1);
        assert!(is_test_function(&t2, &graph));
        let t3 = make_fn("it_should_work", "test.js", 1);
        assert!(is_test_function(&t3, &graph));
        let t4 = make_fn("regular_func", "app.py", 1);
        assert!(!is_test_function(&t4, &graph));
    }

    #[test]
    fn test_is_test_function_by_file_path() {
        let graph = CodeGraph::new();
        let t1 = make_fn("something", "tests/test_app.py", 1);
        assert!(is_test_function(&t1, &graph));
        let t2 = make_fn("something", "src/app.test.ts", 1);
        assert!(is_test_function(&t2, &graph));
        let t3 = make_fn("something", "src/app.spec.js", 1);
        assert!(is_test_function(&t3, &graph));
        // __tests__ needs to be a subdirectory (preceded by /)
        let t4 = make_fn("something", "src/__tests__/App.tsx", 1);
        assert!(is_test_function(&t4, &graph));
        // Regular src file is NOT a test
        let t5 = make_fn("something", "src/app.py", 1);
        assert!(!is_test_function(&t5, &graph));
    }

    #[test]
    fn test_indent() {
        assert_eq!(indent("a\nb", "  "), "  a\n  b");
        assert_eq!(indent("", ">>"), ""); // Empty string has no lines to iterate
        assert_eq!(indent("single", "- "), "- single");
    }

    #[test]
    fn test_format_nodes_empty() {
        let result = format_nodes(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_nodes_with_signature() {
        let mut n = make_fn("greet", "app.py", 1);
        n.signature = Some("def greet(name: str) -> str".to_string());
        let result = format_nodes(&[&n]);
        assert!(result.contains("greet"));
        assert!(result.contains("Signature: def greet"));
    }

    // ── graph_confidence_summary tests ─────────────────────

    #[test]
    fn graph_confidence_summary_empty() {
        let graph = CodeGraph::new();
        let summary = graph_confidence_summary(&graph);
        assert!(summary.contains("0 nodes"));
        assert!(summary.contains("0 edges"));
    }

    #[test]
    fn graph_confidence_summary_with_imports() {
        let mut graph = CodeGraph::new();
        let import = make_import("os", "a.py", 1);
        graph.add_node(import);
        let summary = graph_confidence_summary(&graph);
        assert!(summary.contains("Imports resolved: 0%"));
    }

    #[test]
    fn graph_confidence_summary_with_taint() {
        let mut graph = CodeGraph::new();
        let source = make_fn("src", "a.py", 1);
        let sink = make_fn("sink", "a.py", 10);
        let src_id = source.id;
        let sink_id = sink.id;
        graph.add_node(source);
        graph.add_node(sink);
        graph.add_edge(sink_id, src_id, GirEdge::new(EdgeKind::TaintedBy));
        let summary = graph_confidence_summary(&graph);
        assert!(summary.contains("taint paths"));
    }

    // ── append_confidence_footer tests ─────────────────────

    #[test]
    fn append_footer_adds_graph_info() {
        let graph = CodeGraph::new();
        let session = SessionContext::new();
        let result = CallToolResult::text("Hello".to_string());
        let result = append_confidence_footer(result, &graph, &session);
        assert!(result.content[0].text.contains("Graph:"));
    }

    #[test]
    fn append_footer_includes_session_hints() {
        let mut graph = CodeGraph::new();
        let n1 = make_fn("explored", "a.py", 1);
        let n2 = make_fn("neighbor", "a.py", 10);
        let n1_id = n1.id;
        let n2_id = n2.id;
        graph.add_node(n1);
        graph.add_node(n2);
        graph.add_edge(n2_id, n1_id, GirEdge::new(EdgeKind::Calls));

        let session = SessionContext::new();
        session.record(n1_id, "explored");

        let result = CallToolResult::text("Hello".to_string());
        let result = append_confidence_footer(result, &graph, &session);
        assert!(result.content[0].text.contains("Session:"));
        assert!(result.content[0].text.contains("neighbor"));
    }

    // ── max_results capping tests ──────────────────────────

    #[test]
    fn handle_tool_analyze_dead_code_respects_max() {
        let mut graph = CodeGraph::new();
        for i in 0..10 {
            add_fn_with_parent(&mut graph, &format!("dead_{i}"), "app.py", i * 10 + 1, 0.0);
        }

        let session = SessionContext::new();
        let result = handle_tool(
            "graphy_analyze",
            &json!({"analysis": "dead_code", "max_results": 3}),
            &graph,
            None,
            Path::new("/tmp"),
            &session,
        );
        assert!(result.is_error.is_none());
        // Should show header with 3 candidates
        assert!(result.content[0].text.contains("3 candidates"));
    }
}
