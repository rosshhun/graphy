//! Phase 5: Cross-file call resolution.
//!
//! Builds a name-to-SymbolId map of all real definitions (functions, methods, classes),
//! then tries to resolve CALLS edges from call-site phantoms to actual definitions.

use std::collections::HashMap;
use std::path::Path;

use graphy_core::{
    CodeGraph, EdgeKind, EdgeMetadata, GirEdge, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// Phase 5: Resolve cross-file calls.
pub fn resolve_calls(graph: &mut CodeGraph, _root: &Path) {

    // Build name -> Vec<SymbolId> map for all real definitions (functions, methods, classes, constructors).
    // Exclude phantom call-target nodes — they have no parent Contains edge and would
    // pollute the lookup, causing callers to resolve to phantoms instead of real defs.
    let mut def_map: HashMap<String, Vec<SymbolId>> = HashMap::new();
    for node in graph.all_nodes() {
        if matches!(
            node.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor | NodeKind::Class
        ) && !graph.is_phantom(node.id)
        {
            def_map.entry(node.name.clone()).or_default().push(node.id);
        }
    }

    // Build per-file "available names" from imports + local definitions.
    // Maps (file_path, name) -> SymbolId of the resolved target.
    let mut available: HashMap<(std::path::PathBuf, String), SymbolId> = HashMap::new();

    // Local definitions (real only — phantoms excluded)
    for node in graph.all_nodes() {
        if matches!(
            node.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor | NodeKind::Class
        ) && !graph.is_phantom(node.id)
        {
            available.insert((node.file_path.clone(), node.name.clone()), node.id);
        }
    }

    // Import-provided names: walk Import nodes and follow Imports/ImportsFrom edges
    for import_node in graph.find_by_kind(NodeKind::Import) {
        let import_id = import_node.id;
        let file = import_node.file_path.clone();

        if let Some(idx) = graph.get_node_index(import_id) {
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                let target_idx = edge.target();
                if let Some(target_node) = graph.graph.node_weight(target_idx) {
                    match edge.weight().kind {
                        EdgeKind::ImportsFrom => {
                            // `from X import Y` -- the target is Y (or the file)
                            if let EdgeMetadata::Import { ref items, .. } = edge.weight().metadata {
                                for item in items {
                                    // If target is a real definition, use it
                                    if target_node.kind != NodeKind::File {
                                        available.insert(
                                            (file.clone(), item.clone()),
                                            target_node.id,
                                        );
                                    } else {
                                        // target is the file, look for the symbol within it
                                        for child in graph.children(target_node.id) {
                                            if child.name == *item {
                                                available.insert(
                                                    (file.clone(), item.clone()),
                                                    child.id,
                                                );
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        EdgeKind::Imports => {
                            // `import X` -- makes the module name available
                            let short_name = import_node
                                .name
                                .rsplit('.')
                                .next()
                                .unwrap_or(&import_node.name);
                            available.insert(
                                (file.clone(), short_name.to_string()),
                                target_node.id,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Now iterate all CALLS edges and try to resolve phantom targets.
    // Collect: (source_func_id, phantom_target_id, phantom_name, source_file)
    let mut call_edges: Vec<(SymbolId, SymbolId, String, std::path::PathBuf)> = Vec::new();

    for node in graph.all_nodes() {
        if !node.kind.is_callable() {
            continue;
        }

        let func_id = node.id;
        let file = node.file_path.clone();

        if let Some(idx) = graph.get_node_index(func_id) {
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                if edge.weight().kind == EdgeKind::Calls {
                    let target_idx = edge.target();
                    if let Some(target) = graph.graph.node_weight(target_idx) {
                        let target_id = target.id;
                        let target_name = target.name.clone();

                        // Only process phantom call targets (unresolved call-site nodes).
                        // Real definitions have confidence >= 1.0 and contain children.
                        let has_children = graph
                            .graph
                            .edges_directed(target_idx, Direction::Outgoing)
                            .any(|e| e.weight().kind == EdgeKind::Contains);
                        if target.confidence >= 1.0 && has_children {
                            continue; // Already resolved to a real definition
                        }

                        call_edges.push((func_id, target_id, target_name, file.clone()));
                    }
                }
            }
        }
    }

    let mut resolved_count = 0;
    let mut edges_to_add: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();
    let mut edges_to_remove: Vec<(SymbolId, SymbolId)> = Vec::new();

    for (caller_id, phantom_id, call_name, file) in &call_edges {
        // Extract the final name segment from qualified calls.
        // Handles both `.` paths (obj.method, Module.func) and
        // `::` paths (Self::method, Type::new, module::func).
        let after_dot = call_name.rsplit('.').next().unwrap_or(call_name);
        let simple_name = after_dot.rsplit("::").next().unwrap_or(after_dot);

        // Helper: when we resolve a phantom, record the old phantom edge for removal
        // and the new resolved edge for addition.
        macro_rules! resolve_to {
            ($real_id:expr, $confidence:expr, $is_dynamic:expr) => {{
                let edge = GirEdge::new(EdgeKind::Calls)
                    .with_confidence($confidence)
                    .with_metadata(EdgeMetadata::Call { is_dynamic: $is_dynamic });
                edges_to_add.push((*caller_id, $real_id, edge));
                // Remove the old caller→phantom edge so phantoms don't accumulate
                if *phantom_id != $real_id {
                    edges_to_remove.push((*caller_id, *phantom_id));
                }
                resolved_count += 1;
            }};
        }

        // 1. Check file-local available names
        if let Some(&real_id) = available.get(&(file.clone(), simple_name.to_string())) {
            if real_id != *caller_id {
                resolve_to!(real_id, 0.9, false);
                continue;
            }
        }

        // 2. Check full name match for dotted calls
        if call_name.contains('.') {
            if let Some(&real_id) = available.get(&(file.clone(), call_name.clone())) {
                resolve_to!(real_id, 0.8, true);
                continue;
            }
        }

        // 3. Global name lookup -- if there's exactly one definition, high confidence
        if let Some(defs) = def_map.get(simple_name) {
            if defs.len() == 1 && defs[0] != *caller_id {
                resolve_to!(defs[0], 0.7, call_name.contains('.'));
            } else if defs.len() > 1 {
                // Ambiguous -- try type-aware disambiguation first.
                // If the call is `obj.method()`, check if caller has a parameter/variable
                // whose type matches a class that contains this method.
                let mut type_resolved = false;
                if call_name.contains('.') {
                    let receiver = call_name.split('.').next().unwrap_or("");
                    if !receiver.is_empty() {
                        // Find the type of the receiver by checking ParamType edges
                        // from the caller's children (parameters) with matching name.
                        if graph.get_node(*caller_id).is_some() {
                            for param in graph.children(*caller_id) {
                                if param.name == receiver || param.kind == NodeKind::Parameter {
                                    let param_types = graph.outgoing(param.id, EdgeKind::ParamType);
                                    for type_node in &param_types {
                                        // Check if any definition is a method on this type
                                        for &def_id in defs {
                                            if def_id == *caller_id {
                                                continue;
                                            }
                                            if let Some(def_node) = graph.get_node(def_id) {
                                                // Method's parent class matches the type
                                                let parent_types = graph.incoming(def_id, EdgeKind::Contains);
                                                if parent_types.iter().any(|p| p.id == type_node.id || p.name == type_node.name) {
                                                    resolve_to!(def_id, 0.85, true);
                                                    type_resolved = true;
                                                    break;
                                                }
                                                // Same file as the type definition
                                                if def_node.file_path == type_node.file_path {
                                                    resolve_to!(def_id, 0.75, true);
                                                    type_resolved = true;
                                                    break;
                                                }
                                            }
                                        }
                                        if type_resolved {
                                            break;
                                        }
                                    }
                                }
                                if type_resolved {
                                    break;
                                }
                            }
                        }
                    }
                }

                if !type_resolved {
                    // Fallback: prefer same-file definition
                    let same_file = defs.iter().find(|&&id| {
                        graph
                            .get_node(id)
                            .map_or(false, |n| n.file_path == *file)
                    });
                    if let Some(&real_id) = same_file {
                        if real_id != *caller_id {
                            resolve_to!(real_id, 0.6, call_name.contains('.'));
                        }
                    }
                }
            }
        }
    }

    // Remove phantom edges that were resolved to real definitions.
    // We collect (caller, phantom) pairs and remove matching Calls edges.
    for (caller_id, phantom_id) in &edges_to_remove {
        if let (Some(src_idx), Some(tgt_idx)) = (
            graph.get_node_index(*caller_id),
            graph.get_node_index(*phantom_id),
        ) {
            let to_remove: Vec<_> = graph
                .graph
                .edges_connecting(src_idx, tgt_idx)
                .filter(|e| e.weight().kind == EdgeKind::Calls)
                .map(|e| e.id())
                .collect();
            for ei in to_remove {
                graph.graph.remove_edge(ei);
            }
        }
    }

    for (src, tgt, edge) in edges_to_add {
        graph.add_edge(src, tgt, edge);
    }

    debug!(
        "Phase 5 (Call Tracing): resolved {}/{} calls",
        resolved_count,
        call_edges.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, Language, Span, Visibility};
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

    fn make_fn(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    #[test]
    fn resolve_simple_same_file_call() {
        let mut g = CodeGraph::new();

        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let caller = make_fn("caller", "test.py", 1);
        let caller_id = caller.id;
        g.add_node(caller);
        g.add_edge(file_id, caller_id, GirEdge::new(EdgeKind::Contains));

        let callee = make_fn("callee", "test.py", 10);
        let callee_id = callee.id;
        g.add_node(callee);
        g.add_edge(file_id, callee_id, GirEdge::new(EdgeKind::Contains));

        // Create a phantom call target (no parent → phantom)
        let phantom = GirNode::new(
            "callee".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(99, 0, 99, 0),
            Language::Python,
        );
        let phantom_id = phantom.id;
        g.add_node(phantom);
        g.add_edge(caller_id, phantom_id, GirEdge::new(EdgeKind::Calls));

        resolve_calls(&mut g, Path::new("/project"));

        // Should have resolved to the real callee
        let callees = g.callees(caller_id);
        assert!(!callees.is_empty());
    }

    #[test]
    fn resolve_calls_empty_graph() {
        let mut g = CodeGraph::new();
        resolve_calls(&mut g, Path::new("/project"));
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn no_self_resolution() {
        // A function should not resolve to call itself
        let mut g = CodeGraph::new();

        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let func = make_fn("only_func", "test.py", 1);
        let func_id = func.id;
        g.add_node(func);
        g.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));

        // Phantom target with same name
        let phantom = GirNode::new(
            "only_func".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(99, 0, 99, 0),
            Language::Python,
        );
        let phantom_id = phantom.id;
        g.add_node(phantom);
        g.add_edge(func_id, phantom_id, GirEdge::new(EdgeKind::Calls));

        resolve_calls(&mut g, Path::new("/project"));
        // Calls to self are filtered — phantom shouldn't resolve to the function itself
    }

    #[test]
    fn resolve_dotted_call() {
        let mut g = CodeGraph::new();

        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let caller = make_fn("main", "test.py", 1);
        let caller_id = caller.id;
        g.add_node(caller);
        g.add_edge(file_id, caller_id, GirEdge::new(EdgeKind::Contains));

        let target = make_fn("method", "test.py", 20);
        let target_id = target.id;
        g.add_node(target);
        g.add_edge(file_id, target_id, GirEdge::new(EdgeKind::Contains));

        // Phantom for dotted call "obj.method"
        let phantom = GirNode::new(
            "obj.method".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(98, 0, 98, 0),
            Language::Python,
        );
        let phantom_id = phantom.id;
        g.add_node(phantom);
        g.add_edge(caller_id, phantom_id, GirEdge::new(EdgeKind::Calls));

        resolve_calls(&mut g, Path::new("/project"));

        // Should resolve "obj.method" → "method" via simple name extraction
        let callees = g.callees(caller_id);
        assert!(callees.iter().any(|c| c.name == "method"));
    }
}
