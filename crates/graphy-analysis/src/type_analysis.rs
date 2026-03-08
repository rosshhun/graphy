//! Phase 7: Type reference resolution.
//!
//! Resolve type annotation names (ParamType, ReturnsType, FieldType edges) to actual
//! type definition nodes (Class, Struct, Enum, TypeAlias, etc.) in the graph.

use std::collections::HashMap;

use graphy_core::{
    CodeGraph, EdgeKind, GirEdge, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// Phase 7: Resolve type annotations to real type nodes.
pub fn resolve_types(graph: &mut CodeGraph) {
    // Build name -> Vec<SymbolId> for all type-like definitions
    let mut type_map: HashMap<String, Vec<SymbolId>> = HashMap::new();
    for node in graph.all_nodes() {
        if node.kind.is_type_def() || node.kind == NodeKind::Class {
            type_map.entry(node.name.clone()).or_default().push(node.id);
        }
    }

    // Collect all type annotation edges (ParamType, ReturnsType, FieldType).
    // These currently point to TypeAlias phantom nodes created by the parser.
    // We want to redirect them to real type nodes if possible.
    let type_edge_kinds = [EdgeKind::ParamType, EdgeKind::ReturnsType, EdgeKind::FieldType];

    // Gather: (source_id, phantom_type_id, phantom_type_name, edge_kind)
    let mut type_refs: Vec<(SymbolId, SymbolId, String, EdgeKind)> = Vec::new();

    for node in graph.all_nodes() {
        let node_id = node.id;
        if let Some(idx) = graph.get_node_index(node_id) {
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                if type_edge_kinds.contains(&edge.weight().kind) {
                    let target_idx = edge.target();
                    if let Some(target) = graph.graph.node_weight(target_idx) {
                        // Only resolve if target is a TypeAlias phantom
                        if target.kind == NodeKind::TypeAlias {
                            type_refs.push((
                                node_id,
                                target.id,
                                target.name.clone(),
                                edge.weight().kind,
                            ));
                        }
                    }
                }
            }
        }
    }

    let mut edges_to_add: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();
    let mut resolved_count = 0;

    for (source_id, _phantom_id, type_name, edge_kind) in &type_refs {
        // Clean up the type name: strip Optional[], List[], etc.
        let clean_name = extract_base_type(type_name);

        if clean_name.is_empty() {
            continue;
        }

        // Skip built-in types
        if is_builtin_type(&clean_name) {
            continue;
        }

        if let Some(candidates) = type_map.get(&clean_name) {
            let confidence = if candidates.len() == 1 { 1.0 } else { 0.7 };

            // Prefer same-file type
            let source_file = graph.get_node(*source_id).map(|n| n.file_path.clone());

            let best = candidates
                .iter()
                .find(|&&cid| {
                    graph
                        .get_node(cid)
                        .map_or(false, |n| Some(n.file_path.clone()) == source_file)
                })
                .or_else(|| candidates.first());

            if let Some(&type_id) = best {
                let edge = GirEdge::new(*edge_kind).with_confidence(confidence);
                edges_to_add.push((*source_id, type_id, edge));
                resolved_count += 1;
            }
        }
    }

    for (src, tgt, edge) in edges_to_add {
        graph.add_edge(src, tgt, edge);
    }

    debug!(
        "Phase 7 (Type Analysis): resolved {}/{} type references",
        resolved_count,
        type_refs.len()
    );
}

/// Extract the base type name from a possibly generic annotation.
///   "Optional[Foo]" -> "Foo"
///   "List[Bar]"     -> "Bar"
///   "Dict[str, Foo]" -> "Foo" (takes the last non-builtin)
///   "int"           -> "int"
///   "Foo"           -> "Foo"
fn extract_base_type(type_name: &str) -> String {
    let trimmed = type_name.trim();

    // Handle Optional[], List[], Set[], etc.
    if let Some(inner) = extract_bracket_content(trimmed) {
        // For Dict[K, V], try to find a non-builtin type
        let parts: Vec<&str> = inner.split(',').collect();
        for part in parts.iter().rev() {
            let p = part.trim();
            let base = extract_base_type(p);
            if !base.is_empty() && !is_builtin_type(&base) {
                return base;
            }
        }
        // All are builtins, return first
        if let Some(first) = parts.first() {
            return extract_base_type(first.trim());
        }
    }

    // Handle union types: "Foo | Bar"
    if trimmed.contains('|') {
        for part in trimmed.split('|') {
            let p = part.trim();
            if !is_builtin_type(p) && !p.is_empty() && p != "None" {
                return p.to_string();
            }
        }
    }

    // Plain name
    trimmed.to_string()
}

fn extract_bracket_content(s: &str) -> Option<&str> {
    let open = s.find('[')?;
    let close = s.rfind(']')?;
    if close > open + 1 {
        Some(&s[open + 1..close])
    } else {
        None
    }
}

fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "float"
            | "str"
            | "bool"
            | "bytes"
            | "None"
            | "none"
            | "NoneType"
            | "Any"
            | "object"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "frozenset"
            | "type"
            | "List"
            | "Dict"
            | "Set"
            | "Tuple"
            | "FrozenSet"
            | "Optional"
            | "Union"
            | "Callable"
            | "Iterator"
            | "Generator"
            | "Sequence"
            | "Mapping"
            | "Iterable"
            | "Type"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_type() {
        assert_eq!(extract_base_type("Optional[Foo]"), "Foo");
        assert_eq!(extract_base_type("List[Bar]"), "Bar");
        assert_eq!(extract_base_type("int"), "int");
        assert_eq!(extract_base_type("Foo"), "Foo");
    }

    #[test]
    fn test_is_builtin() {
        assert!(is_builtin_type("int"));
        assert!(is_builtin_type("str"));
        assert!(!is_builtin_type("MyClass"));
    }

    #[test]
    fn test_extract_base_type_union() {
        // Union types: should return first non-builtin
        let result = extract_base_type("int | MyType");
        assert_eq!(result, "MyType");
    }

    #[test]
    fn test_extract_base_type_all_builtins() {
        let result = extract_base_type("int | str | None");
        // All builtins, falls through to plain name
        assert_eq!(result, "int | str | None");
    }

    #[test]
    fn test_extract_base_type_generic() {
        assert_eq!(extract_base_type("Dict[str, MyModel]"), "MyModel");
    }

    #[test]
    fn test_extract_bracket_content() {
        assert_eq!(extract_bracket_content("List[int]"), Some("int"));
        assert_eq!(extract_bracket_content("NoSquareBrackets"), None);
        assert_eq!(extract_bracket_content("[]"), None); // empty brackets
    }

    #[test]
    fn test_resolve_types_empty_graph() {
        let mut graph = graphy_core::CodeGraph::new();
        // Should not panic on empty graph
        resolve_types(&mut graph);
    }
}
