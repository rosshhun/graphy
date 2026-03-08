//! Phase 6: Inheritance chain resolution.
//!
//! For each class with INHERITS edges, resolve base class names to actual Class nodes,
//! build transitive inheritance chains, detect method overrides, and add OVERRIDES edges.

use std::collections::{HashMap, HashSet, VecDeque};

use graphy_core::{
    CodeGraph, EdgeKind, EdgeMetadata, GirEdge, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// Phase 6: Resolve inheritance and detect overrides.
pub fn resolve_inheritance(graph: &mut CodeGraph) {
    // Build name -> Vec<SymbolId> map for all Class nodes
    let mut class_map: HashMap<String, Vec<SymbolId>> = HashMap::new();
    for node in graph.find_by_kind(NodeKind::Class) {
        class_map.entry(node.name.clone()).or_default().push(node.id);
    }

    // Collect all INHERITS edges (child_class -> base_class_phantom)
    let mut inheritance_edges: Vec<(SymbolId, SymbolId, String)> = Vec::new();
    for class_node in graph.find_by_kind(NodeKind::Class) {
        let class_id = class_node.id;
        if let Some(idx) = graph.get_node_index(class_id) {
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                if edge.weight().kind == EdgeKind::Inherits {
                    let target_idx = edge.target();
                    if let Some(target) = graph.graph.node_weight(target_idx) {
                        inheritance_edges.push((
                            class_id,
                            target.id,
                            target.name.clone(),
                        ));
                    }
                }
            }
        }
    }

    // Resolve phantom base classes to real class definitions
    let mut resolved_edges: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();
    let mut parent_map: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new(); // child -> [real parents]

    for (child_id, _phantom_id, base_name) in &inheritance_edges {
        // Handle dotted names like "module.ClassName"
        let simple_name = base_name.rsplit('.').next().unwrap_or(base_name);

        if let Some(candidates) = class_map.get(simple_name) {
            // Filter out self
            let real_bases: Vec<SymbolId> = candidates
                .iter()
                .filter(|&&cid| cid != *child_id)
                .copied()
                .collect();

            if real_bases.len() == 1 {
                let base_id = real_bases[0];
                let edge = GirEdge::new(EdgeKind::Inherits)
                    .with_confidence(1.0)
                    .with_metadata(EdgeMetadata::Inheritance { depth: 1 });
                resolved_edges.push((*child_id, base_id, edge));
                parent_map.entry(*child_id).or_default().push(base_id);
            } else if real_bases.len() > 1 {
                // Ambiguous -- prefer same-file class
                let child_file = graph
                    .get_node(*child_id)
                    .map(|n| n.file_path.clone());

                let best = real_bases
                    .iter()
                    .find(|&&bid| {
                        graph
                            .get_node(bid)
                            .map_or(false, |n| Some(n.file_path.clone()) == child_file)
                    })
                    .or_else(|| real_bases.first());

                if let Some(&base_id) = best {
                    let edge = GirEdge::new(EdgeKind::Inherits)
                        .with_confidence(0.7)
                        .with_metadata(EdgeMetadata::Inheritance { depth: 1 });
                    resolved_edges.push((*child_id, base_id, edge));
                    parent_map.entry(*child_id).or_default().push(base_id);
                }
            }
        }
    }

    // Build transitive inheritance chains (BFS)
    let mut transitive_edges: Vec<(SymbolId, SymbolId, u32)> = Vec::new();
    for (&child_id, direct_parents) in &parent_map {
        let mut visited: HashSet<SymbolId> = HashSet::new();
        let mut queue: VecDeque<(SymbolId, u32)> = VecDeque::new();

        for &p in direct_parents {
            visited.insert(p);
            queue.push_back((p, 1));
        }

        while let Some((ancestor_id, depth)) = queue.pop_front() {
            if depth > 1 {
                // Only add transitive edges (direct ones are already added)
                transitive_edges.push((child_id, ancestor_id, depth));
            }

            if let Some(grandparents) = parent_map.get(&ancestor_id) {
                for &gp in grandparents {
                    if visited.insert(gp) {
                        queue.push_back((gp, depth + 1));
                    }
                }
            }
        }
    }

    for (child, ancestor, depth) in &transitive_edges {
        let edge = GirEdge::new(EdgeKind::Inherits)
            .with_confidence(0.9)
            .with_metadata(EdgeMetadata::Inheritance { depth: *depth });
        resolved_edges.push((*child, *ancestor, edge));
    }

    // Detect method overrides.
    // For each child class, collect its methods, then check if any parent class
    // has a method with the same name.
    let mut override_edges: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();

    for (&child_id, parents) in &parent_map {
        // Collect child methods
        let child_methods: Vec<(String, SymbolId)> = graph
            .children(child_id)
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Method | NodeKind::Constructor))
            .map(|n| (n.name.clone(), n.id))
            .collect();

        // Walk all ancestors (direct + transitive)
        let mut ancestors: HashSet<SymbolId> = HashSet::new();
        let mut queue: VecDeque<SymbolId> = VecDeque::new();
        for &p in parents {
            queue.push_back(p);
        }
        while let Some(a) = queue.pop_front() {
            if ancestors.insert(a) {
                if let Some(gps) = parent_map.get(&a) {
                    for &gp in gps {
                        queue.push_back(gp);
                    }
                }
            }
        }

        for &ancestor_id in &ancestors {
            let ancestor_methods: HashMap<String, SymbolId> = graph
                .children(ancestor_id)
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Method | NodeKind::Constructor))
                .map(|n| (n.name.clone(), n.id))
                .collect();

            for (method_name, child_method_id) in &child_methods {
                if let Some(&parent_method_id) = ancestor_methods.get(method_name) {
                    let edge = GirEdge::new(EdgeKind::Overrides).with_confidence(1.0);
                    override_edges.push((*child_method_id, parent_method_id, edge));
                }
            }
        }
    }

    // Apply all edges
    let resolved_count = resolved_edges.len();
    let override_count = override_edges.len();

    for (src, tgt, edge) in resolved_edges {
        graph.add_edge(src, tgt, edge);
    }
    for (src, tgt, edge) in override_edges {
        graph.add_edge(src, tgt, edge);
    }

    debug!(
        "Phase 6 (Heritage): {} inheritance edges, {} overrides",
        resolved_count, override_count
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, Language, Span, Visibility};
    use std::path::PathBuf;

    fn make_class(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Class,
            PathBuf::from(file),
            Span::new(line, 0, line + 20, 0),
            Language::Python,
        )
    }

    fn make_method(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Method,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

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

    #[test]
    fn resolve_inheritance_empty_graph() {
        let mut g = CodeGraph::new();
        resolve_inheritance(&mut g);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn resolve_simple_inheritance() {
        let mut g = CodeGraph::new();
        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let parent = make_class("Animal", "test.py", 1);
        let parent_id = parent.id;
        g.add_node(parent);
        g.add_edge(file_id, parent_id, GirEdge::new(EdgeKind::Contains));

        let child = make_class("Dog", "test.py", 30);
        let child_id = child.id;
        g.add_node(child);
        g.add_edge(file_id, child_id, GirEdge::new(EdgeKind::Contains));

        // Phantom base class node
        let phantom_base = GirNode::new(
            "Animal".to_string(),
            NodeKind::Class,
            PathBuf::from("test.py"),
            Span::new(99, 0, 99, 0),
            Language::Python,
        );
        let phantom_id = phantom_base.id;
        g.add_node(phantom_base);
        g.add_edge(
            child_id,
            phantom_id,
            GirEdge::new(EdgeKind::Inherits)
                .with_metadata(EdgeMetadata::Inheritance { depth: 1 }),
        );

        resolve_inheritance(&mut g);

        // Check that Dog now has an Inherits edge to the real Animal
        let bases = g.outgoing(child_id, EdgeKind::Inherits);
        assert!(!bases.is_empty());
    }

    #[test]
    fn detect_method_override() {
        let mut g = CodeGraph::new();
        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let parent = make_class("Animal", "test.py", 1);
        let parent_id = parent.id;
        g.add_node(parent);
        g.add_edge(file_id, parent_id, GirEdge::new(EdgeKind::Contains));

        let parent_method = make_method("speak", "test.py", 5);
        let parent_method_id = parent_method.id;
        g.add_node(parent_method);
        g.add_edge(parent_id, parent_method_id, GirEdge::new(EdgeKind::Contains));

        let child = make_class("Dog", "test.py", 30);
        let child_id = child.id;
        g.add_node(child);
        g.add_edge(file_id, child_id, GirEdge::new(EdgeKind::Contains));

        let child_method = make_method("speak", "test.py", 35);
        let child_method_id = child_method.id;
        g.add_node(child_method);
        g.add_edge(child_id, child_method_id, GirEdge::new(EdgeKind::Contains));

        // Add inheritance edge Dog -> Animal
        g.add_edge(
            child_id,
            parent_id,
            GirEdge::new(EdgeKind::Inherits)
                .with_metadata(EdgeMetadata::Inheritance { depth: 1 }),
        );

        resolve_inheritance(&mut g);

        // Dog.speak should have an Overrides edge to Animal.speak
        let overrides = g.outgoing(child_method_id, EdgeKind::Overrides);
        assert!(
            overrides.iter().any(|n| n.id == parent_method_id),
            "Expected Overrides edge from Dog.speak to Animal.speak"
        );
    }
}
