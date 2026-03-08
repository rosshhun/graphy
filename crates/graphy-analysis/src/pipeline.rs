use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use tracing::{info, warn};

use graphy_core::{CodeGraph, EdgeKind, GirEdge, NodeKind, SymbolId};

use crate::discovery::{self, DiscoveredFile};
use crate::{
    call_tracing, change_coupling, community, complexity, dataflow, dead_code, flow_detection,
    heritage, import_resolution, structure, taint, type_analysis,
};

/// Configuration for the analysis pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub incremental: bool,
    pub git_history_months: u32,
    /// Use installed LSP servers for precise call resolution.
    pub use_lsp: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            incremental: true,
            git_history_months: 6,
            use_lsp: false,
        }
    }
}

/// The main analysis pipeline orchestrator.
pub struct AnalysisPipeline {
    config: PipelineConfig,
    root: PathBuf,
}

impl AnalysisPipeline {
    pub fn new(root: PathBuf, config: PipelineConfig) -> Self {
        Self { config, root }
    }

    /// Run the full analysis pipeline and return the populated CodeGraph.
    pub fn run(&self) -> Result<CodeGraph> {
        let total_start = Instant::now();
        let mut graph = CodeGraph::new();

        // Phase 1: File Discovery
        let phase_start = Instant::now();
        let files = discovery::discover_files(&self.root)?;
        info!(
            "Phase 1 (Discovery): {} files in {:?}",
            files.len(),
            phase_start.elapsed()
        );

        if files.is_empty() {
            warn!("No supported source files found in {}", self.root.display());
            return Ok(graph);
        }

        // Check incremental cache
        let cache_path = self.root.join(".graphy").join("hashes.cache");
        let old_hashes = if self.config.incremental {
            discovery::load_hash_cache(&cache_path)
        } else {
            Default::default()
        };

        let files_to_parse: Vec<&DiscoveredFile> = if self.config.incremental {
            files
                .iter()
                .filter(|f| {
                    old_hashes
                        .get(&f.path)
                        .map_or(true, |&old_hash| old_hash != f.content_hash)
                })
                .collect()
        } else {
            files.iter().collect()
        };

        // In incremental mode, detect files that were deleted since last run
        // and remove their stale nodes from the graph
        if self.config.incremental {
            let current_paths: std::collections::HashSet<&std::path::Path> =
                files.iter().map(|f| f.path.as_path()).collect();
            let stale_paths: Vec<std::path::PathBuf> = old_hashes
                .keys()
                .filter(|p| !current_paths.contains(p.as_path()))
                .cloned()
                .collect();
            if !stale_paths.is_empty() {
                info!(
                    "Removing {} deleted files from graph",
                    stale_paths.len()
                );
                for p in &stale_paths {
                    graph.remove_file(p);
                }
            }
        }

        // In incremental mode, remove stale data for CHANGED files before re-parsing.
        // Without this, line-number shifts cause duplicate nodes (old + new SymbolIds).
        if self.config.incremental {
            for f in &files_to_parse {
                graph.remove_file(&f.path);
            }
        }

        info!(
            "{} files changed (of {} total)",
            files_to_parse.len(),
            files.len()
        );

        // Phase 2: Structure Building
        let phase_start = Instant::now();
        structure::build_structure(&mut graph, &self.root, &files);
        info!("Phase 2 (Structure): {:?}", phase_start.elapsed());

        // Phase 3: AST Parsing -> GIR Emission
        let phase_start = Instant::now();
        let file_contents: Vec<(PathBuf, String)> = files_to_parse
            .iter()
            .filter_map(|f| {
                std::fs::read_to_string(&f.path)
                    .ok()
                    .map(|content| (f.path.clone(), content))
            })
            .collect();

        let results = graphy_parser::parse_files(&file_contents);
        let mut parse_errors = 0;

        for (path, result) in results {
            match result {
                Ok(output) => {
                    let file_nodes: Vec<_> = output
                        .nodes
                        .iter()
                        .filter(|n| n.kind == NodeKind::File)
                        .collect();

                    if let Some(file_node) = file_nodes.first() {
                        if let Some(parent_dir) = path.parent() {
                            let folder_name = parent_dir
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy();
                            let folder_id =
                                SymbolId::new(parent_dir, &folder_name, NodeKind::Folder, 0);
                            let file_id = file_node.id;

                            graph.merge(output);
                            graph.add_edge(
                                folder_id,
                                file_id,
                                GirEdge::new(EdgeKind::Contains),
                            );
                        } else {
                            graph.merge(output);
                        }
                    } else {
                        graph.merge(output);
                    }
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                    parse_errors += 1;
                }
            }
        }

        info!(
            "Phase 3 (Parsing): {} files parsed, {} errors, {:?}",
            file_contents.len() - parse_errors,
            parse_errors,
            phase_start.elapsed()
        );

        // Phase 4: Import Resolution
        let phase_start = Instant::now();
        import_resolution::resolve_imports(&mut graph, &self.root);
        info!("Phase 4 (Import Resolution): {:?}", phase_start.elapsed());

        // Phase 5: Call Tracing (cross-file)
        let phase_start = Instant::now();
        call_tracing::resolve_calls(&mut graph, &self.root);
        info!("Phase 5 (Call Tracing): {:?}", phase_start.elapsed());

        // Phase 5.5: Phantom Cleanup — remove unresolved call-target nodes.
        // These are synthetic nodes created during parsing that weren't
        // resolved to real definitions. Language-agnostic: uses structural
        // detection (no parent Contains edge) rather than hardcoded names.
        let phase_start = Instant::now();
        let phantom_count = graph.remove_phantom_nodes();
        info!(
            "Phase 5.5 (Phantom Cleanup): removed {} phantom nodes in {:?}",
            phantom_count,
            phase_start.elapsed()
        );

        // Phase 5.7: LSP Enhancement (optional)
        if self.config.use_lsp {
            let phase_start = Instant::now();
            let lsp_result = crate::lsp_enhance::enhance_with_lsp(&mut graph, &self.root);
            if !lsp_result.servers_used.is_empty() {
                info!(
                    "Phase 5.7 (LSP): {} edges from {} servers ({} functions) in {:?}",
                    lsp_result.edges_added,
                    lsp_result.servers_used.join(", "),
                    lsp_result.functions_queried,
                    phase_start.elapsed()
                );
            } else {
                info!(
                    "Phase 5.7 (LSP): no servers available, skipped in {:?}",
                    phase_start.elapsed()
                );
            }
        }

        // Phase 5.8: Framework Detection
        // Loads built-in configs + custom TOML files from ~/.config/graphy/frameworks/
        let phase_start = Instant::now();
        let custom_fw_dir = graphy_parser::dynamic_loader::grammars_dir()
            .parent()
            .map(|p| p.join("frameworks"))
            .unwrap_or_default();
        let fw_registry = crate::framework::FrameworkRegistry::new()
            .with_custom_dir(&custom_fw_dir);
        let fw_result = fw_registry.analyze(&mut graph, &self.root);
        if !fw_result.frameworks_detected.is_empty() {
            info!(
                "Phase 5.8 (Frameworks): [{}] — {} annotations in {:?}",
                fw_result.frameworks_detected.join(", "),
                fw_result.annotations_added,
                phase_start.elapsed()
            );
        }

        // Phase 6: Heritage Analysis
        let phase_start = Instant::now();
        heritage::resolve_inheritance(&mut graph);
        info!("Phase 6 (Heritage): {:?}", phase_start.elapsed());

        // Phase 7: Type Analysis
        let phase_start = Instant::now();
        type_analysis::resolve_types(&mut graph);
        info!("Phase 7 (Type Analysis): {:?}", phase_start.elapsed());

        // Phase 8: Data Flow Analysis
        let phase_start = Instant::now();
        dataflow::analyze_dataflow(&mut graph);
        info!("Phase 8 (Data Flow): {:?}", phase_start.elapsed());

        // Phase 9: Taint Analysis
        let phase_start = Instant::now();
        let custom_taint = taint::load_custom_taint_rules(&self.root);
        let taint_findings =
            taint::analyze_taint_with_rules(&mut graph, custom_taint.as_ref());
        info!(
            "Phase 9 (Taint): {} findings in {:?}",
            taint_findings.len(),
            phase_start.elapsed()
        );

        // Phase 10: Complexity Metrics
        let phase_start = Instant::now();
        complexity::compute_complexity(&mut graph, &self.root);
        info!("Phase 10 (Complexity): {:?}", phase_start.elapsed());

        // Phase 11: Community Detection
        let phase_start = Instant::now();
        let communities = community::detect_communities(&graph);
        info!(
            "Phase 11 (Communities): {} communities in {:?}",
            communities.len(),
            phase_start.elapsed()
        );

        // Phase 12: Flow Detection
        let phase_start = Instant::now();
        let flows = flow_detection::detect_flows(&mut graph);
        info!(
            "Phase 12 (Flows): {} entry points in {:?}",
            flows.len(),
            phase_start.elapsed()
        );

        // Phase 13: Dead Code Detection
        let phase_start = Instant::now();
        let dead = dead_code::detect_dead_code(&mut graph);
        let dead_count = dead.iter().filter(|d| d.liveness < 0.5).count();
        info!(
            "Phase 13 (Dead Code): {} likely dead in {:?}",
            dead_count,
            phase_start.elapsed()
        );

        // Coverage overlay (optional, if lcov file exists)
        if let Some(report) = crate::coverage::load_coverage(&self.root) {
            crate::coverage::apply_coverage(&mut graph, &report, &self.root);
            info!(
                "Coverage overlay: {}/{} lines covered across {} files",
                report.covered_lines, report.total_lines, report.files.len()
            );
        }

        // Phase 14: Change Coupling (Git History)
        let phase_start = Instant::now();
        change_coupling::analyze_change_coupling(
            &mut graph,
            &self.root,
            self.config.git_history_months,
        );
        info!("Phase 14 (Change Coupling): {:?}", phase_start.elapsed());

        // Save hash cache for incremental indexing
        if self.config.incremental {
            if let Err(e) = discovery::save_hash_cache(&cache_path, &files) {
                warn!("Failed to save hash cache: {}", e);
            }
        }

        info!(
            "Pipeline complete: {} nodes, {} edges in {:?}",
            graph.node_count(),
            graph.edge_count(),
            total_start.elapsed()
        );

        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_config_defaults() {
        let config = PipelineConfig::default();
        assert!(config.incremental);
        assert_eq!(config.git_history_months, 6);
        assert!(!config.use_lsp);
    }

    #[test]
    fn pipeline_on_rust_fixture() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/calibration/rust_project");

        if !fixture.exists() {
            return; // skip if fixture missing
        }

        let config = PipelineConfig {
            incremental: false,
            git_history_months: 0,
            use_lsp: false,
        };
        let pipeline = AnalysisPipeline::new(fixture, config);
        let graph = pipeline.run().unwrap();

        // Should have parsed something
        assert!(graph.node_count() > 0, "graph should have nodes");
        assert!(graph.edge_count() > 0, "graph should have edges");

        // Should have File nodes
        let files = graph.find_by_kind(NodeKind::File);
        assert!(!files.is_empty());

        // Should have Function nodes
        let funcs = graph.find_by_kind(NodeKind::Function);
        assert!(!funcs.is_empty());

        // Graph should be valid after full pipeline
        assert!(graph.validate().is_empty());
    }

    #[test]
    fn pipeline_on_flask_fixture() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/calibration/flask_app");

        if !fixture.exists() {
            return;
        }

        let config = PipelineConfig {
            incremental: false,
            git_history_months: 0,
            use_lsp: false,
        };
        let pipeline = AnalysisPipeline::new(fixture, config);
        let graph = pipeline.run().unwrap();
        assert!(graph.node_count() > 0);
        assert!(graph.validate().is_empty());
    }

    #[test]
    fn pipeline_on_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = PipelineConfig {
            incremental: false,
            git_history_months: 0,
            use_lsp: false,
        };
        let pipeline = AnalysisPipeline::new(tmp.path().to_path_buf(), config);
        let graph = pipeline.run().unwrap();
        // Empty project should have at least a root folder, or 0 nodes
        assert!(graph.validate().is_empty());
    }
}
