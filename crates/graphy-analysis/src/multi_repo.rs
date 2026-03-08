//! Multi-repository analysis.
//!
//! Merges multiple per-repo CodeGraphs into a unified graph with
//! cross-repo edges for shared dependencies and API contracts.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use tracing::info;

use graphy_core::{
    CodeGraph, EdgeKind, GirEdge, GirNode, NodeKind, SymbolId,
    Visibility, storage,
};

use crate::pipeline::{AnalysisPipeline, PipelineConfig};

/// Configuration for multi-repo analysis.
#[derive(Debug, Clone)]
pub struct MultiRepoConfig {
    pub roots: Vec<PathBuf>,
    pub pipeline_config: PipelineConfig,
}

/// Result of multi-repo analysis.
#[derive(Debug)]
pub struct MultiRepoResult {
    pub merged_graph: CodeGraph,
    pub repo_count: usize,
    pub cross_repo_edges: usize,
}

/// Analyze multiple repositories and merge into a unified graph.
pub fn analyze_multi_repo(config: &MultiRepoConfig) -> Result<MultiRepoResult> {
    let mut merged = CodeGraph::new();
    let mut repo_graphs: Vec<(String, CodeGraph)> = Vec::new();

    // Phase 1: Index each repo independently (or load from .redb)
    for root in &config.roots {
        let graph = load_repo_graph(root, &config.pipeline_config)?;
        let repo_name = repo_name_from_path(root);

        info!(
            "Repo {} indexed: {} nodes, {} edges",
            repo_name,
            graph.node_count(),
            graph.edge_count()
        );

        repo_graphs.push((repo_name, graph));
    }

    // Phase 2: Merge graphs with namespaced paths
    for (repo_name, graph) in &repo_graphs {
        merge_graph_with_namespace(&mut merged, graph, repo_name);
    }

    // Phase 3: Detect cross-repo edges
    let cross_repo_edges = detect_cross_repo_edges(&mut merged, &repo_graphs);

    info!(
        "Multi-repo merge complete: {} nodes, {} edges ({} cross-repo)",
        merged.node_count(),
        merged.edge_count(),
        cross_repo_edges
    );

    Ok(MultiRepoResult {
        merged_graph: merged,
        repo_count: repo_graphs.len(),
        cross_repo_edges,
    })
}

/// Derive a repo name from a path (directory name or .redb file stem).
fn repo_name_from_path(path: &PathBuf) -> String {
    if path.extension().map_or(false, |e| e == "redb") {
        // Use the file stem (e.g., "myproject" from "myproject.redb" or "index" from "index.redb")
        // Try the parent directory name for paths like `.graphy/index.redb`
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
        if stem == "index" || stem == "graph" {
            // Use the grandparent directory name instead
            path.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(stem)
        } else {
            stem
        }
    } else {
        std::fs::canonicalize(path)
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// Load a graph for a single repo root. Supports:
/// 1. Direct `.redb` file path
/// 2. Directory with `.graphy/index.redb` (pre-indexed)
/// 3. Directory without index (full pipeline analysis)
fn load_repo_graph(root: &PathBuf, pipeline_config: &PipelineConfig) -> Result<CodeGraph> {
    // Case 1: Path is a .redb file — load directly
    if root.extension().map_or(false, |e| e == "redb") {
        info!("Loading pre-indexed graph from {}", root.display());
        return storage::load_graph(root).map_err(Into::into);
    }

    let canonical = std::fs::canonicalize(root)?;

    // Case 2: Directory with existing .graphy/index.redb
    let db_path = storage::default_db_path(&canonical);
    if db_path.exists() {
        info!(
            "Loading pre-indexed graph for {} from {}",
            canonical.display(),
            db_path.display()
        );
        return storage::load_graph(&db_path).map_err(Into::into);
    }

    // Case 3: No existing index — run the full analysis pipeline
    info!("Indexing repo from scratch: {}", canonical.display());
    let pipeline = AnalysisPipeline::new(canonical, pipeline_config.clone());
    pipeline.run()
}

/// Merge a single repo's graph into the unified graph, namespacing file paths.
fn merge_graph_with_namespace(merged: &mut CodeGraph, graph: &CodeGraph, repo_name: &str) {
    // Re-create nodes with namespaced paths
    let mut id_map: HashMap<SymbolId, SymbolId> = HashMap::new();

    for node in graph.all_nodes() {
        // Normalize: strip leading "/" from absolute paths to avoid PathBuf::join dropping the prefix
        let normalized = node
            .file_path
            .strip_prefix("/")
            .unwrap_or(&node.file_path);
        let namespaced_path = PathBuf::from(repo_name).join(normalized);

        let mut new_node = GirNode::new(
            node.name.clone(),
            node.kind,
            namespaced_path,
            node.span,
            node.language,
        );
        new_node.visibility = node.visibility;
        new_node.signature = node.signature.clone();
        new_node.complexity = node.complexity;
        new_node.confidence = node.confidence;
        new_node.doc = node.doc.clone();

        id_map.insert(node.id, new_node.id);
        merged.add_node(new_node);
    }

    // Re-create edges using the new namespaced IDs
    for node in graph.all_nodes() {
        let edge_kinds = [
            EdgeKind::Contains,
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::ImportsFrom,
            EdgeKind::Inherits,
            EdgeKind::Implements,
            EdgeKind::Overrides,
            EdgeKind::ReturnsType,
            EdgeKind::ParamType,
            EdgeKind::FieldType,
            EdgeKind::Instantiates,
            EdgeKind::DataFlowsTo,
            EdgeKind::TaintedBy,
            EdgeKind::AnnotatedWith,
            EdgeKind::CoupledWith,
        ];

        for edge_kind in &edge_kinds {
            for target in graph.outgoing(node.id, *edge_kind) {
                if let (Some(&new_src), Some(&new_tgt)) =
                    (id_map.get(&node.id), id_map.get(&target.id))
                {
                    merged.add_edge(new_src, new_tgt, GirEdge::new(*edge_kind));
                }
            }
        }
    }
}

/// Detect cross-repo connections based on shared imports and matching APIs.
fn detect_cross_repo_edges(
    merged: &mut CodeGraph,
    _repo_graphs: &[(String, CodeGraph)],
) -> usize {
    let mut cross_edges = 0;

    // Build a map of exported symbol names -> their new IDs in the merged graph
    let mut exports_by_name: HashMap<(String, NodeKind), Vec<SymbolId>> = HashMap::new();

    for node in merged.all_nodes() {
        if matches!(
            node.visibility,
            Visibility::Public | Visibility::Exported
        ) && (node.kind.is_callable() || node.kind.is_type_def())
        {
            exports_by_name
                .entry((node.name.clone(), node.kind))
                .or_default()
                .push(node.id);
        }
    }

    // For each import node, try to match it to an export in another repo
    let import_nodes: Vec<_> = merged
        .all_nodes()
        .filter(|n| n.kind == NodeKind::Import)
        .map(|n| (n.id, n.name.clone(), n.file_path.clone()))
        .collect();

    for (import_id, import_name, import_file) in &import_nodes {
        // Get the repo this import belongs to
        let import_repo = import_file
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();

        // Look for matching exports in other repos
        for kind in &[NodeKind::Function, NodeKind::Class, NodeKind::Struct, NodeKind::Trait] {
            if let Some(export_ids) = exports_by_name.get(&(import_name.clone(), *kind)) {
                for export_id in export_ids {
                    if let Some(export_node) = merged.get_node(*export_id) {
                        let export_repo = export_node
                            .file_path
                            .components()
                            .next()
                            .map(|c| c.as_os_str().to_string_lossy().to_string())
                            .unwrap_or_default();

                        if export_repo != import_repo {
                            merged.add_edge(
                                *import_id,
                                *export_id,
                                GirEdge::new(EdgeKind::CrossLangCalls).with_confidence(0.5),
                            );
                            cross_edges += 1;
                        }
                    }
                }
            }
        }
    }

    cross_edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{Language, Span};

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
    fn repo_name_from_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let name = repo_name_from_path(&tmp.path().to_path_buf());
        assert!(!name.is_empty());
        assert_ne!(name, "unknown");
    }

    #[test]
    fn repo_name_from_redb_file() {
        let name = repo_name_from_path(&PathBuf::from("/projects/myapp/.graphy/index.redb"));
        // "index" should resolve to grandparent "myapp"
        assert_eq!(name, "myapp");
    }

    #[test]
    fn repo_name_from_custom_redb() {
        let name = repo_name_from_path(&PathBuf::from("/data/project.redb"));
        assert_eq!(name, "project");
    }

    #[test]
    fn merge_graph_with_namespace_empty() {
        let mut merged = CodeGraph::new();
        let source = CodeGraph::new();
        merge_graph_with_namespace(&mut merged, &source, "repo_a");
        assert_eq!(merged.node_count(), 0);
    }

    #[test]
    fn merge_graph_with_namespace_renames_paths() {
        let mut merged = CodeGraph::new();
        let mut source = CodeGraph::new();
        source.add_node(make_fn("handler", "src/app.py", 1));

        merge_graph_with_namespace(&mut merged, &source, "my_repo");
        assert_eq!(merged.node_count(), 1);

        let node = merged.all_nodes().next().unwrap();
        assert!(node.file_path.starts_with("my_repo"));
    }

    #[test]
    fn merge_graph_preserves_edges() {
        let mut merged = CodeGraph::new();
        let mut source = CodeGraph::new();
        let a = make_fn("caller", "a.py", 1);
        let b = make_fn("callee", "a.py", 10);
        let a_id = a.id;
        let b_id = b.id;
        source.add_node(a);
        source.add_node(b);
        source.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Calls));

        merge_graph_with_namespace(&mut merged, &source, "repo");
        assert_eq!(merged.node_count(), 2);
        assert!(merged.edge_count() > 0);
    }

    #[test]
    fn detect_cross_repo_edges_matching_import() {
        let mut merged = CodeGraph::new();

        // Repo A exports "process" (public function)
        let mut export = GirNode::new(
            "process".to_string(),
            NodeKind::Function,
            PathBuf::from("repo_a/src/lib.py"),
            Span::new(1, 0, 6, 0),
            Language::Python,
        );
        export.visibility = Visibility::Public;
        merged.add_node(export);

        // Repo B imports "process"
        let import = GirNode::new(
            "process".to_string(),
            NodeKind::Import,
            PathBuf::from("repo_b/src/app.py"),
            Span::new(1, 0, 1, 20),
            Language::Python,
        );
        merged.add_node(import);

        let count = detect_cross_repo_edges(&mut merged, &[]);
        assert_eq!(count, 1);
    }

    #[test]
    fn detect_cross_repo_edges_same_repo_no_edge() {
        let mut merged = CodeGraph::new();

        let mut export = GirNode::new(
            "helper".to_string(),
            NodeKind::Function,
            PathBuf::from("repo_a/src/lib.py"),
            Span::new(1, 0, 6, 0),
            Language::Python,
        );
        export.visibility = Visibility::Public;
        merged.add_node(export);

        // Same repo imports it
        let import = GirNode::new(
            "helper".to_string(),
            NodeKind::Import,
            PathBuf::from("repo_a/src/app.py"),
            Span::new(1, 0, 1, 20),
            Language::Python,
        );
        merged.add_node(import);

        let count = detect_cross_repo_edges(&mut merged, &[]);
        assert_eq!(count, 0);
    }

    #[test]
    fn multi_repo_config_debug() {
        let config = MultiRepoConfig {
            roots: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            pipeline_config: PipelineConfig::default(),
        };
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("/a"));
    }
}
