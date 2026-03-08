//! Phase 11: Community detection via label propagation.
//!
//! Builds an undirected call graph, runs label propagation to convergence,
//! auto-labels communities based on common terms in symbol names, and
//! computes a cohesion score per community.

use std::collections::HashMap;

use graphy_core::{CodeGraph, EdgeKind, NodeKind, SymbolId};
use petgraph::stable_graph::{NodeIndex, StableUnGraph};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// Maximum iterations for label propagation.
const MAX_ITERATIONS: usize = 100;

/// A detected community of related symbols.
#[derive(Debug, Clone)]
pub struct Community {
    pub id: usize,
    pub members: Vec<SymbolId>,
    pub label: String,
    pub cohesion: f64,
}

/// Phase 11: Detect communities in the call graph using label propagation.
pub fn detect_communities(graph: &CodeGraph) -> Vec<Community> {
    // Collect all callable nodes
    let callable_nodes: Vec<(SymbolId, String)> = graph
        .all_nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Constructor | NodeKind::Class
            )
        })
        .map(|n| (n.id, n.name.clone()))
        .collect();

    if callable_nodes.is_empty() {
        return Vec::new();
    }

    // Build an undirected graph for label propagation.
    // Map SymbolId -> local index in the undirected graph.
    let mut sym_to_local: HashMap<SymbolId, NodeIndex> = HashMap::new();
    let mut local_to_sym: HashMap<NodeIndex, SymbolId> = HashMap::new();
    let mut undirected: StableUnGraph<SymbolId, ()> = StableUnGraph::with_capacity(callable_nodes.len(), 0);

    for (sym_id, _name) in &callable_nodes {
        let idx = undirected.add_node(*sym_id);
        sym_to_local.insert(*sym_id, idx);
        local_to_sym.insert(idx, *sym_id);
    }

    // Add edges from the call graph (both directions since undirected)
    for (sym_id, _name) in &callable_nodes {
        if let Some(src_graph_idx) = graph.get_node_index(*sym_id) {
            for edge in graph
                .graph
                .edges_directed(src_graph_idx, Direction::Outgoing)
            {
                if edge.weight().kind == EdgeKind::Calls {
                    let target_graph_idx = edge.target();
                    if let Some(target_node) = graph.graph.node_weight(target_graph_idx) {
                        if let (Some(&src_local), Some(&tgt_local)) = (
                            sym_to_local.get(sym_id),
                            sym_to_local.get(&target_node.id),
                        ) {
                            // Avoid self-loops and duplicate edges
                            if src_local != tgt_local {
                                // Check for existing edge
                                let has_edge = undirected
                                    .edges(src_local)
                                    .any(|e: petgraph::stable_graph::EdgeReference<'_, ()>| e.target() == tgt_local);
                                if !has_edge {
                                    undirected.add_edge(src_local, tgt_local, ());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Label propagation algorithm.
    // Initialize: each node gets its own unique label (its index).
    let node_indices: Vec<NodeIndex> = undirected.node_indices().collect();
    let mut labels: HashMap<NodeIndex, usize> = HashMap::new();
    for (i, &idx) in node_indices.iter().enumerate() {
        labels.insert(idx, i);
    }

    // Iterate until convergence or max iterations.
    for _iter in 0..MAX_ITERATIONS {
        let mut changed = false;

        // Process nodes in a deterministic order
        for &idx in &node_indices {
            let neighbors: Vec<NodeIndex> = undirected.neighbors(idx).collect();
            if neighbors.is_empty() {
                continue;
            }

            // Count labels among neighbors
            let mut label_counts: HashMap<usize, usize> = HashMap::new();
            for &neighbor in &neighbors {
                if let Some(&label) = labels.get(&neighbor) {
                    *label_counts.entry(label).or_insert(0) += 1;
                }
            }

            // Find the most common label
            let best_label = label_counts
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(label, _)| label);

            if let Some(new_label) = best_label {
                let current = labels.get(&idx).copied().unwrap_or(0);
                if new_label != current {
                    labels.insert(idx, new_label);
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    // Group nodes by label
    let mut communities_map: HashMap<usize, Vec<SymbolId>> = HashMap::new();
    for (&idx, &label) in &labels {
        if let Some(&sym_id) = local_to_sym.get(&idx) {
            communities_map.entry(label).or_default().push(sym_id);
        }
    }

    // Build community structs
    let sym_names: HashMap<SymbolId, String> = callable_nodes.into_iter().collect();
    let mut communities: Vec<Community> = Vec::new();

    for (id, members) in &communities_map {
        let label = if members.len() < 2 {
            // Singleton — use the symbol's own name
            members
                .first()
                .and_then(|s| sym_names.get(s))
                .cloned()
                .unwrap_or_else(|| "isolated".into())
        } else {
            auto_label(members, &sym_names)
        };
        let cohesion = compute_cohesion(members, &sym_to_local, &undirected);

        communities.push(Community {
            id: *id,
            members: members.clone(),
            label,
            cohesion,
        });
    }

    // Sort by size descending
    communities.sort_by(|a, b| b.members.len().cmp(&a.members.len()));

    debug!(
        "Phase 11 (Community): {} communities detected ({} nodes total)",
        communities.len(),
        communities.iter().map(|c| c.members.len()).sum::<usize>()
    );

    communities
}

/// Auto-label a community based on common terms in member symbol names.
fn auto_label(members: &[SymbolId], sym_names: &HashMap<SymbolId, String>) -> String {
    let mut term_counts: HashMap<String, usize> = HashMap::new();

    for sym_id in members {
        if let Some(name) = sym_names.get(sym_id) {
            // Split camelCase and snake_case into words
            for word in split_identifier(name) {
                let lower = word.to_lowercase();
                // Skip very short or common words
                if lower.len() >= 3
                    && !matches!(
                        lower.as_str(),
                        "get" | "set" | "new" | "the" | "and" | "for" | "self" | "init" | "test"
                    )
                {
                    *term_counts.entry(lower).or_insert(0) += 1;
                }
            }
        }
    }

    // Pick the top 2 most common terms
    let mut terms: Vec<(String, usize)> = term_counts.into_iter().collect();
    terms.sort_by(|a, b| b.1.cmp(&a.1));

    let top: Vec<&str> = terms.iter().take(2).map(|(t, _)| t.as_str()).collect();
    if top.is_empty() {
        format!("community_{}", members.len())
    } else {
        top.join("_")
    }
}

/// Split a Python identifier into constituent words.
///   "process_user_data" -> ["process", "user", "data"]
///   "processUserData"   -> ["process", "User", "Data"]
fn split_identifier(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in name.chars() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if ch.is_uppercase() && !current.is_empty() {
            words.push(current.clone());
            current.clear();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    words
}

/// Compute cohesion for a community as the ratio of actual internal edges
/// to possible internal edges.
fn compute_cohesion(
    members: &[SymbolId],
    sym_to_local: &HashMap<SymbolId, NodeIndex>,
    undirected: &StableUnGraph<SymbolId, ()>,
) -> f64 {
    let n = members.len();
    if n < 2 {
        return 1.0;
    }

    let member_set: std::collections::HashSet<NodeIndex> = members
        .iter()
        .filter_map(|s| sym_to_local.get(s).copied())
        .collect();

    let mut internal_edges = 0usize;
    for &sym_id in members {
        if let Some(&local_idx) = sym_to_local.get(&sym_id) {
            for neighbor in undirected.neighbors(local_idx) {
                if member_set.contains(&neighbor) {
                    internal_edges += 1;
                }
            }
        }
    }

    // Each edge is counted twice (once from each endpoint)
    internal_edges /= 2;

    let max_edges = n * (n - 1) / 2;
    if max_edges == 0 {
        1.0
    } else {
        internal_edges as f64 / max_edges as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_identifier() {
        assert_eq!(
            split_identifier("process_user_data"),
            vec!["process", "user", "data"]
        );
        assert_eq!(
            split_identifier("processUserData"),
            vec!["process", "User", "Data"]
        );
    }

    #[test]
    fn test_split_identifier_single_word() {
        assert_eq!(split_identifier("main"), vec!["main"]);
    }

    #[test]
    fn test_split_identifier_empty() {
        let result = split_identifier("");
        assert!(result.is_empty() || result == vec![""]);
    }

    #[test]
    fn test_detect_communities_empty_graph() {
        let graph = graphy_core::CodeGraph::new();
        let communities = detect_communities(&graph);
        assert!(communities.is_empty());
    }

    #[test]
    fn test_detect_communities_single_node() {
        let mut graph = graphy_core::CodeGraph::new();
        let node = graphy_core::GirNode::new(
            "isolated_func".to_string(),
            graphy_core::NodeKind::Function,
            std::path::PathBuf::from("app.py"),
            graphy_core::Span::new(1, 0, 5, 0),
            graphy_core::Language::Python,
        );
        graph.add_node(node);
        let communities = detect_communities(&graph);
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[0].members.len(), 1);
    }
}
