//! Trace vulnerable dependency usage to call sites in the code graph.
//!
//! Performs inter-procedural tracing: starting from import nodes that match
//! a dependency, finds direct callers in the same file, then walks up the
//! call graph transitively to reveal the full blast radius.

use std::collections::HashSet;

use serde::Serialize;

use graphy_core::{CodeGraph, EdgeKind, NodeKind, SymbolId};

use crate::DependencyInfo;

/// Default maximum depth for transitive caller tracing.
const DEFAULT_MAX_DEPTH: u32 = 3;

/// A location in the codebase where a dependency is used.
#[derive(Debug, Clone, Serialize)]
pub struct CallSite {
    pub symbol_name: String,
    pub file_path: String,
    pub line: u32,
    pub kind: NodeKind,
    /// Number of hops from the direct import site.
    /// 0 = the import itself, 1 = direct caller of the imported symbol,
    /// 2 = caller of a caller, etc.
    pub depth: u32,
}

/// Trace where a dependency is imported/used in the code graph.
///
/// Looks for Import nodes whose name matches the dependency, finds functions
/// in the same file that reference the imported symbol, then walks the call
/// graph upward (transitively, up to `DEFAULT_MAX_DEPTH` levels) to compute
/// the blast radius.
pub fn trace_dep_usage(dep: &DependencyInfo, graph: &CodeGraph) -> Vec<CallSite> {
    trace_dep_usage_with_depth(dep, graph, DEFAULT_MAX_DEPTH)
}

/// Like [`trace_dep_usage`] but with a configurable maximum trace depth.
fn trace_dep_usage_with_depth(
    dep: &DependencyInfo,
    graph: &CodeGraph,
    max_depth: u32,
) -> Vec<CallSite> {
    let mut call_sites = Vec::new();
    // Track visited symbol IDs to avoid cycles and duplicates
    let mut visited: HashSet<SymbolId> = HashSet::new();

    // Find import nodes that reference this dependency
    let imports = graph.find_by_kind(NodeKind::Import);
    let dep_name_lower = dep.name.to_lowercase();

    let matching_imports: Vec<_> = imports
        .into_iter()
        .filter(|n| {
            let import_name = n.name.to_lowercase();
            // Handle various import patterns:
            // Python: "import requests" -> name = "requests"
            // JS/TS: "import express from 'express'" -> name = "express"
            // Rust: "use serde::Deserialize" -> name might be "serde" or "serde::Deserialize"
            import_name == dep_name_lower
                || import_name.starts_with(&format!("{}.", dep_name_lower))
                || import_name.starts_with(&format!("{}::", dep_name_lower))
                || import_name.starts_with(&format!("{}/", dep_name_lower))
        })
        .collect();

    for import_node in &matching_imports {
        visited.insert(import_node.id);
        call_sites.push(CallSite {
            symbol_name: import_node.name.clone(),
            file_path: import_node.file_path.to_string_lossy().into(),
            line: import_node.span.start_line,
            kind: import_node.kind,
            depth: 0,
        });

        // Find callables in the same file that call/reference the imported symbol.
        // These are the depth-1 nodes — direct users of the dependency.
        let file_nodes = graph.find_by_file(&import_node.file_path);
        for node in file_nodes {
            if !node.kind.is_callable() {
                continue;
            }
            // Check if this function calls anything from the import
            let callees = graph.outgoing(node.id, EdgeKind::Calls);
            let uses_dep = callees.iter().any(|callee| {
                let callee_lower = callee.name.to_lowercase();
                callee_lower == dep_name_lower
                    || callee_lower.starts_with(&format!("{}.", dep_name_lower))
                    || callee_lower.starts_with(&format!("{}::", dep_name_lower))
                    || callee_lower.starts_with(&format!("{}/", dep_name_lower))
            });

            if uses_dep && visited.insert(node.id) {
                call_sites.push(CallSite {
                    symbol_name: node.name.clone(),
                    file_path: node.file_path.to_string_lossy().into(),
                    line: node.span.start_line,
                    kind: node.kind,
                    depth: 1,
                });

                // Walk the call graph upward from this direct user
                if max_depth > 1 {
                    collect_transitive_callers(
                        graph,
                        node.id,
                        2, // next depth level
                        max_depth,
                        &mut visited,
                        &mut call_sites,
                    );
                }
            }
        }
    }

    call_sites
}

/// Recursively collect callers up the call graph using `graph.callers()`.
fn collect_transitive_callers(
    graph: &CodeGraph,
    symbol_id: SymbolId,
    current_depth: u32,
    max_depth: u32,
    visited: &mut HashSet<SymbolId>,
    call_sites: &mut Vec<CallSite>,
) {
    if current_depth > max_depth {
        return;
    }

    let callers = graph.callers(symbol_id);
    for caller in callers {
        if !visited.insert(caller.id) {
            continue; // already visited — skip to avoid cycles
        }

        call_sites.push(CallSite {
            symbol_name: caller.name.clone(),
            file_path: caller.file_path.to_string_lossy().into(),
            line: caller.span.start_line,
            kind: caller.kind,
            depth: current_depth,
        });

        // Continue walking upward if we haven't hit the depth limit
        collect_transitive_callers(
            graph,
            caller.id,
            current_depth + 1,
            max_depth,
            visited,
            call_sites,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirEdge, GirNode, Language, Span, SymbolId, Visibility};
    use std::path::PathBuf;

    fn make_file(name: &str) -> GirNode {
        let path = PathBuf::from(name);
        GirNode {
            id: SymbolId::new(&path, name, NodeKind::File, 0),
            name: name.to_string(),
            kind: NodeKind::File,
            file_path: path,
            span: Span::new(0, 0, 100, 0),
            visibility: Visibility::Public,
            language: Language::Python,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        }
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

    fn make_fn(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    fn make_dep(name: &str) -> DependencyInfo {
        DependencyInfo {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            ecosystem: crate::lockfiles::Ecosystem::Poetry,
            transitive: false,
            parent: None,
        }
    }

    #[test]
    fn trace_no_matches() {
        let g = CodeGraph::new();
        let dep = make_dep("nonexistent");
        let sites = trace_dep_usage(&dep, &g);
        assert!(sites.is_empty());
    }

    #[test]
    fn trace_direct_import() {
        let mut g = CodeGraph::new();
        let file = make_file("app.py");
        let file_id = file.id;
        g.add_node(file);

        let import = make_import("requests", "app.py", 1);
        let import_id = import.id;
        g.add_node(import);
        g.add_edge(file_id, import_id, GirEdge::new(EdgeKind::Contains));

        let dep = make_dep("requests");
        let sites = trace_dep_usage(&dep, &g);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].depth, 0);
        assert_eq!(sites[0].symbol_name, "requests");
    }

    #[test]
    fn trace_with_direct_caller() {
        let mut g = CodeGraph::new();
        let file = make_file("app.py");
        let file_id = file.id;
        g.add_node(file);

        let import = make_import("requests", "app.py", 1);
        let import_id = import.id;
        g.add_node(import);
        g.add_edge(file_id, import_id, GirEdge::new(EdgeKind::Contains));

        // Function that calls requests
        let func = make_fn("fetch_data", "app.py", 5);
        let func_id = func.id;
        g.add_node(func);
        g.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));

        // Phantom call target for requests.get
        let phantom = GirNode::new(
            "requests.get".to_string(),
            NodeKind::Function,
            PathBuf::from("app.py"),
            Span::new(6, 0, 6, 20),
            Language::Python,
        );
        let phantom_id = phantom.id;
        g.add_node(phantom);
        g.add_edge(func_id, phantom_id, GirEdge::new(EdgeKind::Calls));

        let dep = make_dep("requests");
        let sites = trace_dep_usage(&dep, &g);
        // Should find: import (depth 0) + fetch_data (depth 1)
        assert!(sites.len() >= 2);
        assert!(sites.iter().any(|s| s.depth == 0));
        assert!(sites.iter().any(|s| s.depth == 1 && s.symbol_name == "fetch_data"));
    }

    #[test]
    fn trace_cycle_detection() {
        let mut g = CodeGraph::new();
        let file = make_file("app.py");
        let file_id = file.id;
        g.add_node(file);

        let import = make_import("dep", "app.py", 1);
        let import_id = import.id;
        g.add_node(import);
        g.add_edge(file_id, import_id, GirEdge::new(EdgeKind::Contains));

        let f1 = make_fn("a", "app.py", 5);
        let f1_id = f1.id;
        g.add_node(f1);
        g.add_edge(file_id, f1_id, GirEdge::new(EdgeKind::Contains));

        // f1 calls dep
        let phantom = GirNode::new(
            "dep.call".to_string(),
            NodeKind::Function,
            PathBuf::from("app.py"),
            Span::new(6, 0, 6, 10),
            Language::Python,
        );
        let ph_id = phantom.id;
        g.add_node(phantom);
        g.add_edge(f1_id, ph_id, GirEdge::new(EdgeKind::Calls));

        let f2 = make_fn("b", "app.py", 15);
        let f2_id = f2.id;
        g.add_node(f2);
        g.add_edge(file_id, f2_id, GirEdge::new(EdgeKind::Contains));

        // Mutual recursion: b calls a, a calls b
        g.add_edge(f2_id, f1_id, GirEdge::new(EdgeKind::Calls));
        g.add_edge(f1_id, f2_id, GirEdge::new(EdgeKind::Calls));

        let dep = make_dep("dep");
        let sites = trace_dep_usage(&dep, &g);
        // Should not infinite loop — cycle detection via visited set
        assert!(!sites.is_empty());
    }
}
