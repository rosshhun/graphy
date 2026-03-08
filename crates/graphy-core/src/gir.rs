use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::symbol_id::SymbolId;

// ── Node Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum NodeKind {
    Module,
    File,
    Folder,
    Class,
    Struct,
    Enum,
    Interface,
    Trait,
    Function,
    Method,
    Constructor,
    Field,
    Property,
    Parameter,
    Variable,
    Constant,
    TypeAlias,
    Import,
    Decorator,
    EnumVariant,
}

impl NodeKind {
    pub fn is_callable(&self) -> bool {
        matches!(
            self,
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor
        )
    }

    pub fn is_type_def(&self) -> bool {
        matches!(
            self,
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Enum
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::TypeAlias
        )
    }
}

// ── Edge Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    Contains,
    Calls,
    Imports,
    ImportsFrom,
    Inherits,
    Implements,
    Overrides,
    ReturnsType,
    ParamType,
    FieldType,
    Instantiates,
    DataFlowsTo,
    TaintedBy,
    CrossLangCalls,
    AnnotatedWith,
    CoupledWith,
    SimilarTo,
}

// ── Visibility ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Visibility {
    Public,
    #[default]
    Internal,
    Private,
    Exported,
}

// ── Language ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Python,
    TypeScript,
    JavaScript,
    Rust,
    Go,
    Java,
    Cpp,
    C,
    CSharp,
    Ruby,
    Kotlin,
    Php,
    Svelte,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Language::Python),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "rs" => Some(Language::Rust),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "cpp" | "cc" | "cxx" | "hpp" => Some(Language::Cpp),
            "c" | "h" => Some(Language::C),
            "cs" => Some(Language::CSharp),
            "rb" => Some(Language::Ruby),
            "kt" => Some(Language::Kotlin),
            "php" => Some(Language::Php),
            "svelte" => Some(Language::Svelte),
            _ => None,
        }
    }
}

// ── Span ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Span {
    pub fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

// ── Complexity Metrics ──────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    pub cyclomatic: u32,
    pub cognitive: u32,
    pub loc: u32,
    pub sloc: u32,
    pub parameter_count: u32,
    pub max_nesting_depth: u32,
}

// ── GIR Node ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GirNode {
    pub id: SymbolId,
    pub name: String,
    pub kind: NodeKind,
    pub file_path: PathBuf,
    pub span: Span,
    pub visibility: Visibility,
    pub language: Language,
    pub signature: Option<String>,
    pub complexity: Option<ComplexityMetrics>,
    pub confidence: f32,
    pub doc: Option<String>,
    /// Test coverage (0.0-1.0), set by coverage overlay. None if no coverage data.
    pub coverage: Option<f32>,
}

impl GirNode {
    pub fn new(
        name: String,
        kind: NodeKind,
        file_path: PathBuf,
        span: Span,
        language: Language,
    ) -> Self {
        let id = SymbolId::new(&file_path, &name, kind, span.start_line);
        Self {
            id,
            name,
            kind,
            file_path,
            span,
            visibility: Visibility::default(),
            language,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        }
    }
}

// ── Edge Metadata ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeMetadata {
    None,
    Call {
        is_dynamic: bool,
    },
    Import {
        alias: Option<String>,
        items: Vec<String>,
    },
    Inheritance {
        depth: u32,
    },
    DataFlow {
        transform: DataFlowTransform,
    },
    Taint {
        label: String,
    },
    Coupling {
        commit_count: u32,
        temporal_weight: f64,
    },
    Similarity {
        score: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataFlowTransform {
    Identity,
    Map,
    Filter,
    Serialize,
    Deserialize,
    Validate,
    Transform,
}

// ── GIR Edge ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GirEdge {
    pub kind: EdgeKind,
    pub confidence: f32,
    pub metadata: EdgeMetadata,
}

impl GirEdge {
    pub fn new(kind: EdgeKind) -> Self {
        Self {
            kind,
            confidence: 1.0,
            metadata: EdgeMetadata::None,
        }
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_metadata(mut self, metadata: EdgeMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

// ── Parse Output ────────────────────────────────────────────

/// The output of parsing a single file: a collection of nodes and edges.
#[derive(Debug, Clone, Default)]
pub struct ParseOutput {
    pub nodes: Vec<GirNode>,
    pub edges: Vec<(SymbolId, SymbolId, GirEdge)>,
}

impl ParseOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: GirNode) {
        self.nodes.push(node);
    }

    pub fn add_edge(&mut self, source: SymbolId, target: SymbolId, edge: GirEdge) {
        self.edges.push((source, target, edge));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── NodeKind ───────────────────────────────────────────

    #[test]
    fn callable_kinds() {
        assert!(NodeKind::Function.is_callable());
        assert!(NodeKind::Method.is_callable());
        assert!(NodeKind::Constructor.is_callable());
        assert!(!NodeKind::Class.is_callable());
        assert!(!NodeKind::Field.is_callable());
        assert!(!NodeKind::Variable.is_callable());
        assert!(!NodeKind::Import.is_callable());
        assert!(!NodeKind::Module.is_callable());
    }

    #[test]
    fn type_def_kinds() {
        assert!(NodeKind::Class.is_type_def());
        assert!(NodeKind::Struct.is_type_def());
        assert!(NodeKind::Enum.is_type_def());
        assert!(NodeKind::Interface.is_type_def());
        assert!(NodeKind::Trait.is_type_def());
        assert!(NodeKind::TypeAlias.is_type_def());
        assert!(!NodeKind::Function.is_type_def());
        assert!(!NodeKind::Method.is_type_def());
        assert!(!NodeKind::Field.is_type_def());
        assert!(!NodeKind::Decorator.is_type_def());
    }

    // ── Language ───────────────────────────────────────────

    #[test]
    fn language_from_common_extensions() {
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("java"), Some(Language::Java));
        assert_eq!(Language::from_extension("svelte"), Some(Language::Svelte));
        assert_eq!(Language::from_extension("php"), Some(Language::Php));
        assert_eq!(Language::from_extension("rb"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("kt"), Some(Language::Kotlin));
    }

    #[test]
    fn language_from_variant_extensions() {
        // TS variants
        assert_eq!(Language::from_extension("mts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("cts"), Some(Language::TypeScript));
        // JS variants
        assert_eq!(Language::from_extension("mjs"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("cjs"), Some(Language::JavaScript));
        // C/C++ variants
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("h"), Some(Language::C));
        assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cc"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cs"), Some(Language::CSharp));
    }

    #[test]
    fn language_from_unknown_extension() {
        assert_eq!(Language::from_extension(""), None);
        assert_eq!(Language::from_extension("txt"), None);
        assert_eq!(Language::from_extension("md"), None);
        assert_eq!(Language::from_extension("toml"), None);
        assert_eq!(Language::from_extension("PY"), None); // case-sensitive
    }

    // ── Span ──────────────────────────────────────────────

    #[test]
    fn span_new() {
        let s = Span::new(1, 0, 10, 80);
        assert_eq!(s.start_line, 1);
        assert_eq!(s.start_col, 0);
        assert_eq!(s.end_line, 10);
        assert_eq!(s.end_col, 80);
    }

    #[test]
    fn span_zero() {
        let s = Span::new(0, 0, 0, 0);
        assert_eq!(s.start_line, 0);
        assert_eq!(s.end_line, 0);
    }

    #[test]
    fn span_max_values() {
        let s = Span::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert_eq!(s.start_line, u32::MAX);
    }

    // ── Visibility ────────────────────────────────────────

    #[test]
    fn visibility_default_is_internal() {
        assert_eq!(Visibility::default(), Visibility::Internal);
    }

    // ── GirNode ───────────────────────────────────────────

    #[test]
    fn gir_node_defaults() {
        let node = GirNode::new(
            "foo".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(1, 0, 5, 0),
            Language::Python,
        );
        assert_eq!(node.name, "foo");
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.visibility, Visibility::Internal);
        assert_eq!(node.confidence, 1.0);
        assert!(node.signature.is_none());
        assert!(node.complexity.is_none());
        assert!(node.doc.is_none());
        assert!(node.coverage.is_none());
    }

    #[test]
    fn gir_node_unicode_name() {
        let node = GirNode::new(
            "计算_résultat".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(1, 0, 5, 0),
            Language::Python,
        );
        assert_eq!(node.name, "计算_résultat");
        // ID should still be deterministic
        let node2 = GirNode::new(
            "计算_résultat".to_string(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(1, 0, 5, 0),
            Language::Python,
        );
        assert_eq!(node.id, node2.id);
    }

    #[test]
    fn gir_node_empty_name() {
        let node = GirNode::new(
            String::new(),
            NodeKind::Function,
            PathBuf::from("test.py"),
            Span::new(1, 0, 5, 0),
            Language::Python,
        );
        assert_eq!(node.name, "");
    }

    // ── GirEdge ───────────────────────────────────────────

    #[test]
    fn gir_edge_builder_pattern() {
        let edge = GirEdge::new(EdgeKind::Calls)
            .with_confidence(0.8)
            .with_metadata(EdgeMetadata::Call { is_dynamic: true });
        assert_eq!(edge.kind, EdgeKind::Calls);
        assert_eq!(edge.confidence, 0.8);
        match edge.metadata {
            EdgeMetadata::Call { is_dynamic } => assert!(is_dynamic),
            _ => panic!("expected Call metadata"),
        }
    }

    #[test]
    fn gir_edge_default_metadata() {
        let edge = GirEdge::new(EdgeKind::Contains);
        assert_eq!(edge.confidence, 1.0);
        assert!(matches!(edge.metadata, EdgeMetadata::None));
    }

    // ── Serde round-trips ─────────────────────────────────

    #[test]
    fn node_kind_serde_round_trip() {
        for kind in [
            NodeKind::Module, NodeKind::File, NodeKind::Folder,
            NodeKind::Class, NodeKind::Struct, NodeKind::Enum,
            NodeKind::Interface, NodeKind::Trait, NodeKind::Function,
            NodeKind::Method, NodeKind::Constructor, NodeKind::Field,
            NodeKind::Property, NodeKind::Parameter, NodeKind::Variable,
            NodeKind::Constant, NodeKind::TypeAlias, NodeKind::Import,
            NodeKind::Decorator, NodeKind::EnumVariant,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: NodeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn edge_kind_serde_round_trip() {
        for kind in [
            EdgeKind::Contains, EdgeKind::Calls, EdgeKind::Imports,
            EdgeKind::ImportsFrom, EdgeKind::Inherits, EdgeKind::Implements,
            EdgeKind::Overrides, EdgeKind::ReturnsType, EdgeKind::ParamType,
            EdgeKind::FieldType, EdgeKind::Instantiates, EdgeKind::DataFlowsTo,
            EdgeKind::TaintedBy, EdgeKind::CrossLangCalls, EdgeKind::AnnotatedWith,
            EdgeKind::CoupledWith, EdgeKind::SimilarTo,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: EdgeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn edge_metadata_serde_round_trip() {
        let cases: Vec<EdgeMetadata> = vec![
            EdgeMetadata::None,
            EdgeMetadata::Call { is_dynamic: true },
            EdgeMetadata::Call { is_dynamic: false },
            EdgeMetadata::Import { alias: Some("a".into()), items: vec!["x".into(), "y".into()] },
            EdgeMetadata::Import { alias: None, items: vec![] },
            EdgeMetadata::Inheritance { depth: 0 },
            EdgeMetadata::Inheritance { depth: u32::MAX },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Identity },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Map },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Filter },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Serialize },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Deserialize },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Validate },
            EdgeMetadata::DataFlow { transform: DataFlowTransform::Transform },
            EdgeMetadata::Taint { label: "XSS".into() },
            EdgeMetadata::Coupling { commit_count: 42, temporal_weight: 0.95 },
            EdgeMetadata::Similarity { score: 0.87 },
        ];
        for meta in cases {
            let json = serde_json::to_string(&meta).unwrap();
            let _back: EdgeMetadata = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn gir_node_serde_round_trip() {
        let mut node = GirNode::new(
            "test_func".to_string(),
            NodeKind::Function,
            PathBuf::from("src/main.rs"),
            Span::new(10, 4, 25, 1),
            Language::Rust,
        );
        node.visibility = Visibility::Public;
        node.signature = Some("fn test_func(x: i32) -> bool".into());
        node.doc = Some("A test function".into());
        node.coverage = Some(0.75);
        node.confidence = 0.9;

        let json = serde_json::to_string(&node).unwrap();
        let back: GirNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, node.id);
        assert_eq!(back.name, "test_func");
        assert_eq!(back.kind, NodeKind::Function);
        assert_eq!(back.visibility, Visibility::Public);
        assert_eq!(back.confidence, 0.9);
        assert_eq!(back.signature.as_deref(), Some("fn test_func(x: i32) -> bool"));
        assert_eq!(back.coverage, Some(0.75));
    }

    #[test]
    fn gir_node_bincode_round_trip() {
        let node = GirNode::new(
            "my_func".to_string(),
            NodeKind::Method,
            PathBuf::from("lib.py"),
            Span::new(1, 0, 100, 0),
            Language::Python,
        );
        let bytes = bincode::serialize(&node).unwrap();
        let back: GirNode = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.id, node.id);
        assert_eq!(back.name, node.name);
    }

    #[test]
    fn gir_edge_bincode_round_trip() {
        let edge = GirEdge::new(EdgeKind::Calls)
            .with_confidence(0.7)
            .with_metadata(EdgeMetadata::Call { is_dynamic: true });
        let bytes = bincode::serialize(&edge).unwrap();
        let back: GirEdge = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.kind, EdgeKind::Calls);
        assert_eq!(back.confidence, 0.7);
    }

    // ── ParseOutput ───────────────────────────────────────

    #[test]
    fn parse_output_empty() {
        let po = ParseOutput::new();
        assert!(po.nodes.is_empty());
        assert!(po.edges.is_empty());
    }

    #[test]
    fn parse_output_add_node_and_edge() {
        let mut po = ParseOutput::new();
        let node = GirNode::new(
            "f".to_string(),
            NodeKind::Function,
            PathBuf::from("a.py"),
            Span::new(1, 0, 5, 0),
            Language::Python,
        );
        let id = node.id;
        po.add_node(node);
        assert_eq!(po.nodes.len(), 1);

        let node2 = GirNode::new(
            "g".to_string(),
            NodeKind::Function,
            PathBuf::from("a.py"),
            Span::new(10, 0, 15, 0),
            Language::Python,
        );
        let id2 = node2.id;
        po.add_node(node2);
        po.add_edge(id, id2, GirEdge::new(EdgeKind::Calls));
        assert_eq!(po.edges.len(), 1);
    }

    // ── ComplexityMetrics ─────────────────────────────────

    #[test]
    fn complexity_metrics_default() {
        let m = ComplexityMetrics::default();
        assert_eq!(m.cyclomatic, 0);
        assert_eq!(m.cognitive, 0);
        assert_eq!(m.loc, 0);
        assert_eq!(m.sloc, 0);
        assert_eq!(m.parameter_count, 0);
        assert_eq!(m.max_nesting_depth, 0);
    }
}
