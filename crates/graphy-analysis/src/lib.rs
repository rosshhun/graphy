pub mod discovery;
pub mod pipeline;
pub mod structure;

// Analysis phases
pub mod import_resolution; // Phase 4: Import resolution
pub mod call_tracing; // Phase 5: Cross-file call resolution
pub mod heritage; // Phase 6: Inheritance chains
pub mod type_analysis; // Phase 7: Type reference resolution
pub mod dataflow; // Phase 8: Data flow analysis
pub mod taint; // Phase 9: Taint analysis
pub mod complexity; // Phase 10: Complexity metrics
pub mod community; // Phase 11: Community detection
pub mod flow_detection; // Phase 12: Process/flow detection
pub mod dead_code; // Phase 13: Dead code detection
pub mod change_coupling; // Phase 14: Git history analysis

// LSP enhancement (optional, uses installed language servers)
pub mod lsp_enhance;

// Framework plugin system (auto-detects WordPress, React, Django, etc.)
pub mod framework;

// M7: Advanced features
pub mod context_gen; // Codified context generation
pub mod coverage; // Test coverage overlay (lcov/cobertura)
pub mod multi_repo; // Multi-repository analysis

pub use pipeline::AnalysisPipeline;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_pipeline_constructible() {
        let config = pipeline::PipelineConfig::default();
        let _pipeline = AnalysisPipeline::new(std::path::PathBuf::from("/tmp"), config);
    }

    #[test]
    fn modules_are_accessible() {
        // Verify all phase modules are accessible through the re-exports
        // These module paths should compile without error
        let _ = std::any::type_name::<pipeline::AnalysisPipeline>();
    }

    #[test]
    fn resolve_imports_empty_graph() {
        let mut graph = graphy_core::CodeGraph::new();
        import_resolution::resolve_imports(&mut graph, std::path::Path::new("/tmp"));
        assert_eq!(graph.node_count(), 0);
    }
}
