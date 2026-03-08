use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::gir::{EdgeKind, GirEdge, GirNode, NodeKind, ParseOutput};
use crate::symbol_id::SymbolId;

/// Serializable representation of the graph (nodes + edges as flat vecs).
#[derive(Serialize, Deserialize)]
struct SerializableGraph {
    nodes: Vec<GirNode>,
    edges: Vec<(SymbolId, SymbolId, GirEdge)>,
}

/// The central code knowledge graph.
///
/// Wraps a petgraph StableGraph with indexes for fast lookups by ID, name, and file.
pub struct CodeGraph {
    pub graph: StableGraph<GirNode, GirEdge>,
    id_index: HashMap<SymbolId, NodeIndex>,
    name_index: HashMap<String, Vec<NodeIndex>>,
    file_index: HashMap<PathBuf, Vec<NodeIndex>>,
    kind_index: HashMap<NodeKind, Vec<NodeIndex>>,
}

impl std::fmt::Debug for CodeGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeGraph")
            .field("nodes", &self.graph.node_count())
            .field("edges", &self.graph.edge_count())
            .finish()
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for CodeGraph {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let nodes: Vec<GirNode> = self.graph.node_weights().cloned().collect();
        let edges: Vec<(SymbolId, SymbolId, GirEdge)> = self
            .graph
            .edge_indices()
            .filter_map(|ei| {
                let (src_idx, tgt_idx) = self.graph.edge_endpoints(ei)?;
                let src_node = self.graph.node_weight(src_idx)?;
                let tgt_node = self.graph.node_weight(tgt_idx)?;
                let edge = self.graph.edge_weight(ei)?;
                Some((src_node.id, tgt_node.id, edge.clone()))
            })
            .collect();

        let sg = SerializableGraph { nodes, edges };
        sg.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CodeGraph {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let sg = SerializableGraph::deserialize(deserializer)?;
        let mut graph = CodeGraph::new();
        for node in sg.nodes {
            graph.add_node(node);
        }
        let mut dropped_edges = 0u64;
        for (src, tgt, edge) in sg.edges {
            if !graph.add_edge(src, tgt, edge) {
                dropped_edges += 1;
            }
        }
        if dropped_edges > 0 {
            warn!(
                "Deserialization: dropped {} edges referencing missing nodes",
                dropped_edges
            );
        }
        Ok(graph)
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            id_index: HashMap::new(),
            name_index: HashMap::new(),
            file_index: HashMap::new(),
            kind_index: HashMap::new(),
        }
    }

    // ── Mutation ─────────────────────────────────────────

    pub fn add_node(&mut self, node: GirNode) -> NodeIndex {
        let id = node.id;
        let name = node.name.clone();
        let file = node.file_path.clone();
        let kind = node.kind;

        // Deduplicate: if a node with this ID already exists, update it.
        // Clean up stale secondary index entries if name/kind/file changed.
        if let Some(&existing) = self.id_index.get(&id) {
            if let Some(old) = self.graph.node_weight(existing) {
                let old_name = old.name.clone();
                let old_kind = old.kind;
                let old_file = old.file_path.clone();

                // Update secondary indexes if values changed
                if old_name != name {
                    if let Some(names) = self.name_index.get_mut(&old_name) {
                        names.retain(|i| *i != existing);
                        if names.is_empty() {
                            self.name_index.remove(&old_name);
                        }
                    }
                    self.name_index.entry(name).or_default().push(existing);
                }
                if old_kind != kind {
                    if let Some(kinds) = self.kind_index.get_mut(&old_kind) {
                        kinds.retain(|i| *i != existing);
                        if kinds.is_empty() {
                            self.kind_index.remove(&old_kind);
                        }
                    }
                    self.kind_index.entry(kind).or_default().push(existing);
                }
                if old_file != file {
                    if let Some(files) = self.file_index.get_mut(&old_file) {
                        files.retain(|i| *i != existing);
                        if files.is_empty() {
                            self.file_index.remove(&old_file);
                        }
                    }
                    self.file_index.entry(file).or_default().push(existing);
                }
            }
            self.graph[existing] = node;
            return existing;
        }

        let idx = self.graph.add_node(node);
        self.id_index.insert(id, idx);
        self.name_index.entry(name).or_default().push(idx);
        self.file_index.entry(file).or_default().push(idx);
        self.kind_index.entry(kind).or_default().push(idx);
        idx
    }

    pub fn add_edge(&mut self, source: SymbolId, target: SymbolId, edge: GirEdge) -> bool {
        let (Some(&src_idx), Some(&tgt_idx)) =
            (self.id_index.get(&source), self.id_index.get(&target))
        else {
            return false;
        };
        // Deduplicate: skip if an edge of the same kind already exists between these nodes
        let dominated = self
            .graph
            .edges_connecting(src_idx, tgt_idx)
            .any(|e| e.weight().kind == edge.kind);
        if dominated {
            return true;
        }
        self.graph.add_edge(src_idx, tgt_idx, edge);
        true
    }

    /// Merge a ParseOutput into the graph.
    pub fn merge(&mut self, output: ParseOutput) {
        for node in output.nodes {
            self.add_node(node);
        }
        for (src, tgt, edge) in output.edges {
            self.add_edge(src, tgt, edge);
        }
    }

    /// Remove all nodes belonging to a file (for incremental re-indexing).
    ///
    /// petgraph's `StableGraph::remove_node()` also removes all edges
    /// incident to the node, so no dangling edges are left behind.
    pub fn remove_file(&mut self, path: &Path) {
        let indices: Vec<NodeIndex> = self.file_index.remove(path).unwrap_or_default();

        for idx in &indices {
            if let Some(node) = self.graph.node_weight(*idx) {
                let id = node.id;
                let name = node.name.clone();
                let kind = node.kind;

                self.id_index.remove(&id);
                if let Some(names) = self.name_index.get_mut(&name) {
                    names.retain(|i| i != idx);
                    if names.is_empty() {
                        self.name_index.remove(&name);
                    }
                }
                if let Some(kinds) = self.kind_index.get_mut(&kind) {
                    kinds.retain(|i| i != idx);
                    if kinds.is_empty() {
                        self.kind_index.remove(&kind);
                    }
                }
            }
            // This also removes all edges incident to the node.
            self.graph.remove_node(*idx);
        }

        // Note: we intentionally do NOT shrink_to_fit() here.
        // In watch mode, remove_file is immediately followed by merge (re-add),
        // so shrinking would cause unnecessary reallocation churn.
    }

    /// Validate graph invariants. Returns a list of issues found.
    ///
    /// Checks that all indexes point to valid nodes and that no stale
    /// entries exist. Useful for debugging and post-load verification.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Check id_index entries point to valid nodes with matching IDs
        for (&id, &idx) in &self.id_index {
            match self.graph.node_weight(idx) {
                Some(node) => {
                    if node.id != id {
                        errors.push(format!(
                            "id_index mismatch: key={} but node.id={}",
                            id, node.id
                        ));
                    }
                }
                None => {
                    errors.push(format!("id_index stale: {} -> {:?} (no node)", id, idx));
                }
            }
        }

        // Check name_index entries point to valid nodes
        for (name, indices) in &self.name_index {
            for idx in indices {
                if self.graph.node_weight(*idx).is_none() {
                    errors.push(format!(
                        "name_index stale: '{}' -> {:?} (no node)",
                        name, idx
                    ));
                }
            }
        }

        // Check file_index entries point to valid nodes
        for (path, indices) in &self.file_index {
            for idx in indices {
                if self.graph.node_weight(*idx).is_none() {
                    errors.push(format!(
                        "file_index stale: '{}' -> {:?} (no node)",
                        path.display(),
                        idx
                    ));
                }
            }
        }

        // Check kind_index entries point to valid nodes
        for (kind, indices) in &self.kind_index {
            for idx in indices {
                if self.graph.node_weight(*idx).is_none() {
                    errors.push(format!(
                        "kind_index stale: {:?} -> {:?} (no node)",
                        kind, idx
                    ));
                }
            }
        }

        // Check all graph nodes are in the id_index
        for idx in self.graph.node_indices() {
            if let Some(node) = self.graph.node_weight(idx) {
                if !self.id_index.contains_key(&node.id) {
                    errors.push(format!(
                        "node {} not in id_index (idx={:?})",
                        node.id, idx
                    ));
                }
            }
        }

        errors
    }

    // ── Queries ─────────────────────────────────────────

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn get_node(&self, id: SymbolId) -> Option<&GirNode> {
        self.id_index
            .get(&id)
            .and_then(|idx| self.graph.node_weight(*idx))
    }

    pub fn get_node_index(&self, id: SymbolId) -> Option<NodeIndex> {
        self.id_index.get(&id).copied()
    }

    pub fn find_by_name(&self, name: &str) -> Vec<&GirNode> {
        self.name_index
            .get(name)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn find_by_file(&self, path: &Path) -> Vec<&GirNode> {
        self.file_index
            .get(path)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn find_by_kind(&self, kind: NodeKind) -> Vec<&GirNode> {
        self.kind_index
            .get(&kind)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all nodes connected by outgoing edges of a specific kind.
    pub fn outgoing(&self, id: SymbolId, edge_kind: EdgeKind) -> Vec<&GirNode> {
        let Some(&idx) = self.id_index.get(&id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .filter(|e| e.weight().kind == edge_kind)
            .filter_map(|e| self.graph.node_weight(e.target()))
            .collect()
    }

    /// Get all nodes connected by incoming edges of a specific kind.
    pub fn incoming(&self, id: SymbolId, edge_kind: EdgeKind) -> Vec<&GirNode> {
        let Some(&idx) = self.id_index.get(&id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .filter(|e| e.weight().kind == edge_kind)
            .filter_map(|e| self.graph.node_weight(e.source()))
            .collect()
    }

    /// Get callees of a function/method.
    pub fn callees(&self, id: SymbolId) -> Vec<&GirNode> {
        self.outgoing(id, EdgeKind::Calls)
    }

    /// Get callers of a function/method.
    pub fn callers(&self, id: SymbolId) -> Vec<&GirNode> {
        self.incoming(id, EdgeKind::Calls)
    }

    /// Get children (contained symbols) of a node.
    pub fn children(&self, id: SymbolId) -> Vec<&GirNode> {
        self.outgoing(id, EdgeKind::Contains)
    }

    /// Get the parent (container) of a node.
    pub fn parent(&self, id: SymbolId) -> Option<&GirNode> {
        self.incoming(id, EdgeKind::Contains).into_iter().next()
    }

    /// Check if a node is a "phantom" call target — i.e. created by the parser
    /// to represent a call expression, not an actual symbol definition.
    /// Phantom nodes have no parent (no incoming Contains edge) and are not
    /// top-level File/Folder/Module nodes.
    pub fn is_phantom(&self, id: SymbolId) -> bool {
        let Some(&idx) = self.id_index.get(&id) else {
            return false;
        };
        let Some(node) = self.graph.node_weight(idx) else {
            return false;
        };
        // File, Folder, Module nodes are legitimate roots without parents
        if matches!(node.kind, NodeKind::File | NodeKind::Folder | NodeKind::Module) {
            return false;
        }
        self.parent(id).is_none()
    }

    /// Check if a callable node is an interface/trait implementation method.
    /// Works across all languages: checks whether the method's parent type
    /// has any Implements or Inherits edges, meaning its methods may be
    /// called via dynamic dispatch and shouldn't be flagged as dead.
    pub fn is_interface_impl(&self, id: SymbolId) -> bool {
        let Some(parent) = self.parent(id) else {
            return false;
        };
        let parent_id = parent.id;
        !self.outgoing(parent_id, EdgeKind::Implements).is_empty()
            || !self.outgoing(parent_id, EdgeKind::Inherits).is_empty()
    }

    /// Structural check: is this a method on a type that is actively used?
    ///
    /// Methods called via `self.method()` / `this.method()` are filtered at
    /// parse time to avoid noise, so they have no incoming `Calls` edges.
    /// Instead of hardcoding self/this dispatch, we use a structural signal:
    /// if the parent type (struct/class/enum) has incoming edges (it's
    /// constructed or referenced), its private methods are likely alive
    /// via self-dispatch.
    pub fn is_method_on_used_type(&self, id: SymbolId) -> bool {
        let Some(parent) = self.parent(id) else {
            return false;
        };
        if !matches!(
            parent.kind,
            NodeKind::Struct | NodeKind::Class | NodeKind::Enum | NodeKind::Trait
        ) {
            return false;
        }
        let Some(idx) = self.get_node_index(parent.id) else {
            return false;
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .any(|e| e.weight().kind != EdgeKind::Contains)
    }

    /// Structural check: does this symbol (or a parent module) have any
    /// decorator / attribute annotations?
    ///
    /// This is a language-agnostic heuristic for framework-registered symbols
    /// (tests, route handlers, event listeners, DI-managed beans, etc.).
    /// Instead of hardcoding per-language name prefixes like `test_`, we rely
    /// on the graph structure: if the symbol (or an ancestor module) carries
    /// an `AnnotatedWith` edge to a `Decorator` node, some framework knows
    /// about it and it should not be considered dead code.
    pub fn is_decorated(&self, id: SymbolId) -> bool {
        // Direct decorator on this symbol
        if !self.outgoing(id, EdgeKind::AnnotatedWith).is_empty() {
            return true;
        }

        // Propagate through parent Module / File nodes.
        // A function inside `#[cfg(test)] mod tests { ... }` inherits
        // the module's decorator status, but a method inside a struct
        // with `#[derive(Debug)]` does NOT — derive is metadata, not
        // a framework registration for the struct's methods.
        if let Some(parent) = self.parent(id) {
            if matches!(parent.kind, NodeKind::Module | NodeKind::File) {
                return self.is_decorated(parent.id);
            }
        }
        false
    }

    /// Remove orphaned phantom nodes — phantoms with no incoming edges at all.
    /// Keeps phantom nodes that are call targets (have incoming Calls edges)
    /// since they represent useful callee information in context views.
    pub fn remove_phantom_nodes(&mut self) -> usize {
        let phantoms: Vec<(SymbolId, NodeIndex)> = self
            .id_index
            .iter()
            .filter_map(|(&id, &idx)| {
                if !self.is_phantom(id) {
                    return None;
                }
                // Keep phantom nodes that have incoming edges (they're referenced)
                let has_incoming = self
                    .graph
                    .edges_directed(idx, Direction::Incoming)
                    .next()
                    .is_some();
                if has_incoming {
                    return None;
                }
                Some((id, idx))
            })
            .collect();

        let count = phantoms.len();
        for (id, idx) in &phantoms {
            if let Some(node) = self.graph.node_weight(*idx) {
                let name = node.name.clone();
                let kind = node.kind;
                let file = node.file_path.clone();
                if let Some(names) = self.name_index.get_mut(&name) {
                    names.retain(|i| i != idx);
                    if names.is_empty() {
                        self.name_index.remove(&name);
                    }
                }
                if let Some(kinds) = self.kind_index.get_mut(&kind) {
                    kinds.retain(|i| i != idx);
                    if kinds.is_empty() {
                        self.kind_index.remove(&kind);
                    }
                }
                if let Some(files) = self.file_index.get_mut(&file) {
                    files.retain(|i| i != idx);
                    if files.is_empty() {
                        self.file_index.remove(&file);
                    }
                }
            }
            self.id_index.remove(id);
            self.graph.remove_node(*idx);
        }
        count
    }

    /// All indexed file paths.
    pub fn indexed_files(&self) -> Vec<&PathBuf> {
        self.file_index.keys().collect()
    }

    /// Iterator over all nodes.
    pub fn all_nodes(&self) -> impl Iterator<Item = &GirNode> {
        self.graph.node_weights()
    }

    /// Mutable iterator over all nodes.
    pub fn all_nodes_mut(&mut self) -> impl Iterator<Item = &mut GirNode> {
        self.graph.node_weights_mut()
    }

    /// Remove all edges of a given kind from the entire graph.
    /// Used to clear analysis-phase edges before re-running analysis.
    pub fn remove_edges_by_kind(&mut self, kind: EdgeKind) -> usize {
        let to_remove: Vec<_> = self
            .graph
            .edge_indices()
            .filter(|&ei| {
                self.graph
                    .edge_weight(ei)
                    .map_or(false, |e| e.kind == kind)
            })
            .collect();
        let count = to_remove.len();
        for ei in to_remove {
            self.graph.remove_edge(ei);
        }
        count
    }

    /// Remove all edges of any of the given kinds from the entire graph.
    pub fn remove_edges_by_kinds(&mut self, kinds: &[EdgeKind]) -> usize {
        let to_remove: Vec<_> = self
            .graph
            .edge_indices()
            .filter(|&ei| {
                self.graph
                    .edge_weight(ei)
                    .map_or(false, |e| kinds.contains(&e.kind))
            })
            .collect();
        let count = to_remove.len();
        for ei in to_remove {
            self.graph.remove_edge(ei);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gir::{Language, Span};

    fn make_node(name: &str, kind: NodeKind, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            kind,
            PathBuf::from("test.py"),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    #[test]
    fn add_and_query_nodes() {
        let mut g = CodeGraph::new();
        let func = make_node("my_func", NodeKind::Function, 1);
        let id = func.id;
        g.add_node(func);

        assert_eq!(g.node_count(), 1);
        assert!(g.get_node(id).is_some());
        assert_eq!(g.find_by_name("my_func").len(), 1);
    }

    #[test]
    fn add_edges_and_query() {
        let mut g = CodeGraph::new();
        let caller = make_node("caller", NodeKind::Function, 1);
        let callee = make_node("callee", NodeKind::Function, 10);
        let caller_id = caller.id;
        let callee_id = callee.id;

        g.add_node(caller);
        g.add_node(callee);
        g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));

        assert_eq!(g.callees(caller_id).len(), 1);
        assert_eq!(g.callers(callee_id).len(), 1);
        assert_eq!(g.callees(caller_id)[0].name, "callee");
    }

    #[test]
    fn deduplication() {
        let mut g = CodeGraph::new();
        let n1 = make_node("foo", NodeKind::Function, 1);
        let n2 = make_node("foo", NodeKind::Function, 1);
        g.add_node(n1);
        g.add_node(n2);
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn remove_file() {
        let mut g = CodeGraph::new();
        let n1 = make_node("foo", NodeKind::Function, 1);
        let n2 = make_node("bar", NodeKind::Function, 10);
        g.add_node(n1);
        g.add_node(n2);
        assert_eq!(g.node_count(), 2);

        g.remove_file(Path::new("test.py"));
        assert_eq!(g.node_count(), 0);
    }

    #[test]
    fn remove_file_cleans_edges() {
        let mut g = CodeGraph::new();
        let caller = make_node("caller", NodeKind::Function, 1);
        let callee = GirNode::new(
            "callee".to_string(),
            NodeKind::Function,
            PathBuf::from("other.py"),
            Span::new(1, 0, 6, 0),
            Language::Python,
        );
        let caller_id = caller.id;
        let callee_id = callee.id;
        g.add_node(caller);
        g.add_node(callee);
        g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));
        assert_eq!(g.edge_count(), 1);

        // Remove file containing caller — edge should be cleaned up
        g.remove_file(Path::new("test.py"));
        assert_eq!(g.edge_count(), 0);
        assert_eq!(g.node_count(), 1); // callee in other.py survives
        assert!(g.validate().is_empty());
    }

    #[test]
    fn validate_clean_graph() {
        let mut g = CodeGraph::new();
        let n = make_node("foo", NodeKind::Function, 1);
        g.add_node(n);
        assert!(g.validate().is_empty());
    }

    #[test]
    fn edge_deduplication() {
        let mut g = CodeGraph::new();
        let caller = make_node("caller", NodeKind::Function, 1);
        let callee = make_node("callee", NodeKind::Function, 10);
        let caller_id = caller.id;
        let callee_id = callee.id;
        g.add_node(caller);
        g.add_node(callee);

        // First edge should succeed
        assert!(g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls)));
        assert_eq!(g.edge_count(), 1);

        // Same edge kind between same nodes should be deduped
        assert!(g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls)));
        assert_eq!(g.edge_count(), 1); // Still 1, not 2

        // Different edge kind between same nodes should be allowed
        assert!(g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Contains)));
        assert_eq!(g.edge_count(), 2);
    }

    #[test]
    fn remove_edges_by_kind() {
        let mut g = CodeGraph::new();
        let a = make_node("a", NodeKind::Function, 1);
        let b = make_node("b", NodeKind::Function, 10);
        let c = make_node("c", NodeKind::Class, 20);
        let a_id = a.id;
        let b_id = b.id;
        let c_id = c.id;
        g.add_node(a);
        g.add_node(b);
        g.add_node(c);

        g.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Calls));
        g.add_edge(a_id, c_id, GirEdge::new(EdgeKind::Imports));
        g.add_edge(b_id, c_id, GirEdge::new(EdgeKind::Calls));
        assert_eq!(g.edge_count(), 3);

        // Remove only Calls edges
        let removed = g.remove_edges_by_kind(EdgeKind::Calls);
        assert_eq!(removed, 2);
        assert_eq!(g.edge_count(), 1); // Only Imports edge remains
    }

    #[test]
    fn serialization_round_trip() {
        let mut g = CodeGraph::new();
        let caller = make_node("caller", NodeKind::Function, 1);
        let callee = make_node("callee", NodeKind::Function, 10);
        let caller_id = caller.id;
        let callee_id = callee.id;
        g.add_node(caller);
        g.add_node(callee);
        g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));

        let bytes = bincode::serialize(&g).unwrap();
        let g2: CodeGraph = bincode::deserialize(&bytes).unwrap();

        assert_eq!(g2.node_count(), 2);
        assert_eq!(g2.edge_count(), 1);
        assert_eq!(g2.callees(caller_id).len(), 1);
        assert!(g2.validate().is_empty());
    }

    // ── Edge case tests ───────────────────────────────────

    #[test]
    fn empty_graph() {
        let g = CodeGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
        assert!(g.validate().is_empty());
        assert!(g.find_by_name("anything").is_empty());
        assert!(g.find_by_file(Path::new("anything.rs")).is_empty());
        assert!(g.find_by_kind(NodeKind::Function).is_empty());
        assert!(g.indexed_files().is_empty());
    }

    #[test]
    fn add_edge_missing_nodes() {
        let mut g = CodeGraph::new();
        let fake_id1 = SymbolId::new(Path::new("a.py"), "x", NodeKind::Function, 1);
        let fake_id2 = SymbolId::new(Path::new("a.py"), "y", NodeKind::Function, 2);
        // Should return false when nodes don't exist
        assert!(!g.add_edge(fake_id1, fake_id2, GirEdge::new(EdgeKind::Calls)));
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn add_edge_one_node_missing() {
        let mut g = CodeGraph::new();
        let node = make_node("exists", NodeKind::Function, 1);
        let id = node.id;
        g.add_node(node);
        let fake_id = SymbolId::new(Path::new("a.py"), "ghost", NodeKind::Function, 99);
        assert!(!g.add_edge(id, fake_id, GirEdge::new(EdgeKind::Calls)));
        assert!(!g.add_edge(fake_id, id, GirEdge::new(EdgeKind::Calls)));
    }

    #[test]
    fn find_by_kind_query() {
        let mut g = CodeGraph::new();
        g.add_node(make_node("f1", NodeKind::Function, 1));
        g.add_node(make_node("f2", NodeKind::Function, 10));
        g.add_node(make_node("C", NodeKind::Class, 20));
        assert_eq!(g.find_by_kind(NodeKind::Function).len(), 2);
        assert_eq!(g.find_by_kind(NodeKind::Class).len(), 1);
        assert_eq!(g.find_by_kind(NodeKind::Method).len(), 0);
    }

    #[test]
    fn get_nonexistent_node() {
        let g = CodeGraph::new();
        let fake_id = SymbolId::new(Path::new("a.py"), "x", NodeKind::Function, 1);
        assert!(g.get_node(fake_id).is_none());
        assert!(g.get_node_index(fake_id).is_none());
    }

    #[test]
    fn outgoing_incoming_nonexistent_node() {
        let g = CodeGraph::new();
        let fake_id = SymbolId::new(Path::new("a.py"), "x", NodeKind::Function, 1);
        assert!(g.outgoing(fake_id, EdgeKind::Calls).is_empty());
        assert!(g.incoming(fake_id, EdgeKind::Calls).is_empty());
        assert!(g.callees(fake_id).is_empty());
        assert!(g.callers(fake_id).is_empty());
        assert!(g.children(fake_id).is_empty());
        assert!(g.parent(fake_id).is_none());
    }

    #[test]
    fn phantom_detection() {
        let mut g = CodeGraph::new();
        // File node — NOT phantom (top-level)
        let file = GirNode::new(
            "test.py".to_string(),
            NodeKind::File,
            PathBuf::from("test.py"),
            Span::new(0, 0, 100, 0),
            Language::Python,
        );
        let file_id = file.id;
        g.add_node(file);
        assert!(!g.is_phantom(file_id));

        // Function with parent (Contains edge) — NOT phantom
        let func = make_node("real_func", NodeKind::Function, 5);
        let func_id = func.id;
        g.add_node(func);
        g.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));
        assert!(!g.is_phantom(func_id));

        // Function WITHOUT parent — IS phantom
        let phantom = make_node("phantom_call", NodeKind::Function, 99);
        let phantom_id = phantom.id;
        g.add_node(phantom);
        assert!(g.is_phantom(phantom_id));
    }

    #[test]
    fn remove_phantom_nodes_cleans_indexes() {
        let mut g = CodeGraph::new();
        let file = GirNode::new(
            "test.py".to_string(),
            NodeKind::File,
            PathBuf::from("test.py"),
            Span::new(0, 0, 100, 0),
            Language::Python,
        );
        let file_id = file.id;
        g.add_node(file);

        let real = make_node("real", NodeKind::Function, 1);
        let real_id = real.id;
        g.add_node(real);
        g.add_edge(file_id, real_id, GirEdge::new(EdgeKind::Contains));

        // Orphan phantom (no incoming edges at all)
        let orphan = make_node("orphan", NodeKind::Function, 50);
        g.add_node(orphan);

        // Referenced phantom (has incoming Calls edge — should be kept)
        let referenced = make_node("referenced", NodeKind::Function, 60);
        let ref_id = referenced.id;
        g.add_node(referenced);
        g.add_edge(real_id, ref_id, GirEdge::new(EdgeKind::Calls));

        assert_eq!(g.node_count(), 4);
        let removed = g.remove_phantom_nodes();
        assert_eq!(removed, 1); // Only orphan removed
        assert_eq!(g.node_count(), 3);
        assert!(g.validate().is_empty()); // All indexes consistent
    }

    #[test]
    fn remove_file_nonexistent() {
        let mut g = CodeGraph::new();
        g.add_node(make_node("f", NodeKind::Function, 1));
        g.remove_file(Path::new("nonexistent.py"));
        assert_eq!(g.node_count(), 1); // unchanged
        assert!(g.validate().is_empty());
    }

    #[test]
    fn merge_parse_output() {
        let mut g = CodeGraph::new();
        let mut po = ParseOutput::new();
        let n1 = make_node("a", NodeKind::Function, 1);
        let n2 = make_node("b", NodeKind::Function, 10);
        let id1 = n1.id;
        let id2 = n2.id;
        po.add_node(n1);
        po.add_node(n2);
        po.add_edge(id1, id2, GirEdge::new(EdgeKind::Calls));
        g.merge(po);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert!(g.validate().is_empty());
    }

    #[test]
    fn add_node_update_changes_name() {
        let mut g = CodeGraph::new();
        let mut n = make_node("old_name", NodeKind::Function, 1);
        let id = n.id;
        g.add_node(n.clone());
        assert_eq!(g.find_by_name("old_name").len(), 1);

        // Re-add with same id but different name
        n.name = "new_name".to_string();
        g.add_node(n);
        assert_eq!(g.node_count(), 1); // Still 1 node
        assert_eq!(g.find_by_name("old_name").len(), 0); // Old name cleaned
        assert_eq!(g.find_by_name("new_name").len(), 1); // New name indexed
        assert_eq!(g.get_node(id).unwrap().name, "new_name");
        assert!(g.validate().is_empty());
    }

    #[test]
    fn self_edge() {
        let mut g = CodeGraph::new();
        let func = make_node("recursive", NodeKind::Function, 1);
        let id = func.id;
        g.add_node(func);
        assert!(g.add_edge(id, id, GirEdge::new(EdgeKind::Calls)));
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.callees(id).len(), 1);
        assert_eq!(g.callers(id).len(), 1);
    }

    #[test]
    fn remove_edges_by_kinds_multiple() {
        let mut g = CodeGraph::new();
        let a = make_node("a", NodeKind::Function, 1);
        let b = make_node("b", NodeKind::Function, 10);
        let a_id = a.id;
        let b_id = b.id;
        g.add_node(a);
        g.add_node(b);
        g.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Calls));
        g.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Imports));
        g.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Contains));
        assert_eq!(g.edge_count(), 3);

        let removed = g.remove_edges_by_kinds(&[EdgeKind::Calls, EdgeKind::Imports]);
        assert_eq!(removed, 2);
        assert_eq!(g.edge_count(), 1); // Only Contains remains
    }

    #[test]
    fn is_interface_impl_check() {
        let mut g = CodeGraph::new();
        let file = GirNode::new(
            "test.py".to_string(),
            NodeKind::File,
            PathBuf::from("test.py"),
            Span::new(0, 0, 100, 0),
            Language::Python,
        );
        let file_id = file.id;
        g.add_node(file);

        let cls = make_node("MyClass", NodeKind::Class, 1);
        let cls_id = cls.id;
        g.add_node(cls);
        g.add_edge(file_id, cls_id, GirEdge::new(EdgeKind::Contains));

        let method = make_node("my_method", NodeKind::Method, 5);
        let method_id = method.id;
        g.add_node(method);
        g.add_edge(cls_id, method_id, GirEdge::new(EdgeKind::Contains));

        // No implements/inherits → not an interface impl
        assert!(!g.is_interface_impl(method_id));

        // Add Implements edge to class
        let iface = make_node("MyInterface", NodeKind::Interface, 50);
        let iface_id = iface.id;
        g.add_node(iface);
        g.add_edge(cls_id, iface_id, GirEdge::new(EdgeKind::Implements));

        // Now method IS on an implementing type
        assert!(g.is_interface_impl(method_id));
    }

    #[test]
    fn serialization_round_trip_empty_graph() {
        let g = CodeGraph::new();
        let bytes = bincode::serialize(&g).unwrap();
        let g2: CodeGraph = bincode::deserialize(&bytes).unwrap();
        assert_eq!(g2.node_count(), 0);
        assert_eq!(g2.edge_count(), 0);
        assert!(g2.validate().is_empty());
    }

    #[test]
    fn serialization_drops_edges_with_missing_nodes() {
        // Manually craft a SerializableGraph with dangling edge
        let node = make_node("only_node", NodeKind::Function, 1);
        let fake_id = SymbolId::new(Path::new("x.py"), "ghost", NodeKind::Function, 99);
        let sg = serde_json::json!({
            "nodes": [serde_json::to_value(&node).unwrap()],
            "edges": [[
                serde_json::to_value(node.id).unwrap(),
                serde_json::to_value(fake_id).unwrap(),
                serde_json::to_value(GirEdge::new(EdgeKind::Calls)).unwrap()
            ]]
        });
        let g: CodeGraph = serde_json::from_value(sg).unwrap();
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0); // dangling edge dropped
        assert!(g.validate().is_empty());
    }

    #[test]
    fn indexed_files_returns_correct_paths() {
        let mut g = CodeGraph::new();
        g.add_node(make_node("f1", NodeKind::Function, 1)); // test.py
        let other = GirNode::new(
            "f2".to_string(),
            NodeKind::Function,
            PathBuf::from("other.py"),
            Span::new(1, 0, 6, 0),
            Language::Python,
        );
        g.add_node(other);
        let files = g.indexed_files();
        assert_eq!(files.len(), 2);
        let paths: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
        assert!(paths.contains(&Path::new("test.py")));
        assert!(paths.contains(&Path::new("other.py")));
    }
}
