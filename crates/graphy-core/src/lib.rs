pub mod diff;
pub mod error;
pub mod gir;
pub mod graph;
pub mod storage;
pub mod symbol_id;

pub use error::GraphyError;
pub use gir::*;
pub use graph::CodeGraph;
pub use symbol_id::SymbolId;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexports_available() {
        // Verify key re-exports work
        let _graph = CodeGraph::new();
        let _id = SymbolId::new(std::path::Path::new("test.rs"), "test", NodeKind::Function, 1);
        let _err = GraphyError::Storage("test".into());
    }

    #[test]
    fn gir_types_reexported() {
        // GirNode, GirEdge, NodeKind, EdgeKind, Span should all be available
        let span = Span::new(1, 0, 10, 0);
        let node = GirNode::new(
            "test".into(),
            NodeKind::Function,
            std::path::PathBuf::from("test.rs"),
            span,
            Language::Rust,
        );
        let _edge = GirEdge::new(EdgeKind::Calls);
        assert_eq!(node.name, "test");
    }

    #[test]
    fn parse_output_reexported() {
        let _output = ParseOutput::new();
    }
}
