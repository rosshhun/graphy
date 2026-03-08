use std::path::Path;

use anyhow::{Context, Result};
use graphy_core::{
    EdgeKind, EdgeMetadata, GirEdge, GirNode, NodeKind, ParseOutput, SymbolId, Visibility,
};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::frontend::LanguageFrontend;
use crate::helpers::{clean_doc_comment, node_span, node_text};
use crate::tags_registry::TagsLanguageConfig;

pub struct TagsFrontend {
    config: TagsLanguageConfig,
    query: Option<Query>,
}

impl TagsFrontend {
    pub fn new(config: TagsLanguageConfig) -> Self {
        let query = match Query::new(&config.ts_language, &config.tags_query) {
            Ok(q) => Some(q),
            Err(e) => {
                tracing::warn!(
                    language = ?config.language,
                    error = %e,
                    "Failed to compile tags query; will produce file node only"
                );
                None
            }
        };
        Self { config, query }
    }
}

/// Resolved capture indices for the tags query.
struct CaptureIndices {
    name: Option<u32>,
    doc: Option<u32>,
    definition_function: Option<u32>,
    definition_method: Option<u32>,
    definition_class: Option<u32>,
    definition_interface: Option<u32>,
    definition_module: Option<u32>,
    definition_constant: Option<u32>,
    definition_decorator: Option<u32>,
    reference_call: Option<u32>,
}

impl CaptureIndices {
    fn from_query(query: &Query) -> Self {
        let mut indices = CaptureIndices {
            name: None,
            doc: None,
            definition_function: None,
            definition_method: None,
            definition_class: None,
            definition_interface: None,
            definition_module: None,
            definition_constant: None,
            definition_decorator: None,
            reference_call: None,
        };
        for (i, cap_name) in query.capture_names().iter().enumerate() {
            let idx = i as u32;
            match *cap_name {
                "name" => indices.name = Some(idx),
                "doc" => indices.doc = Some(idx),
                "definition.function" => indices.definition_function = Some(idx),
                "definition.method" => indices.definition_method = Some(idx),
                "definition.class" => indices.definition_class = Some(idx),
                "definition.interface" => indices.definition_interface = Some(idx),
                "definition.module" => indices.definition_module = Some(idx),
                "definition.constant" => indices.definition_constant = Some(idx),
                "definition.decorator" => indices.definition_decorator = Some(idx),
                "reference.call" => indices.reference_call = Some(idx),
                _ => {}
            }
        }
        indices
    }

    fn kind_for_capture(&self, capture_index: u32) -> Option<NodeKind> {
        if self.definition_function == Some(capture_index) {
            Some(NodeKind::Function)
        } else if self.definition_method == Some(capture_index) {
            Some(NodeKind::Method)
        } else if self.definition_class == Some(capture_index) {
            Some(NodeKind::Class)
        } else if self.definition_interface == Some(capture_index) {
            Some(NodeKind::Interface)
        } else if self.definition_module == Some(capture_index) {
            Some(NodeKind::Module)
        } else if self.definition_constant == Some(capture_index) {
            Some(NodeKind::Constant)
        } else {
            None
        }
    }
}

/// A definition extracted from tags query matches.
struct DefEntry {
    id: SymbolId,
    start_byte: usize,
    end_byte: usize,
}

/// A decorator/annotation extracted from tags query matches.
struct DecoratorEntry {
    name: String,
    byte_pos: usize,
    span: graphy_core::Span,
}

/// A call reference extracted from tags query matches.
struct CallEntry {
    name: String,
    call_start_byte: usize,
    call_start_line: u32,
    call_start_col: u32,
    call_end_line: u32,
    call_end_col: u32,
}

impl LanguageFrontend for TagsFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.config.ts_language)
            .context("Failed to set language for tags frontend")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse returned None")?;

        let root = tree.root_node();
        let mut output = ParseOutput::new();
        let source_bytes = source.as_bytes();

        // Create the file node
        let file_node = GirNode {
            id: SymbolId::new(path, path.to_string_lossy().as_ref(), NodeKind::File, 0),
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            kind: NodeKind::File,
            file_path: path.to_path_buf(),
            span: node_span(&root),
            visibility: Visibility::Public,
            language: self.config.language,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        };
        let file_id = file_node.id;
        output.add_node(file_node);

        // If query compilation failed, return just the file node
        let Some(query) = &self.query else {
            return Ok(output);
        };

        let indices = CaptureIndices::from_query(query);
        let mut cursor = QueryCursor::new();

        // Collect definitions, calls, and decorators from query matches
        let mut definitions: Vec<DefEntry> = Vec::new();
        let mut calls: Vec<CallEntry> = Vec::new();
        let mut decorators: Vec<DecoratorEntry> = Vec::new();

        let mut matches = cursor.matches(query, root, source_bytes);
        while let Some(m) = matches.next() {
            let mut name_text: Option<String> = None;
            let mut def_kind: Option<NodeKind> = None;
            let mut def_node: Option<tree_sitter::Node> = None;
            let mut is_call = false;
            let mut is_decorator = false;
            let mut call_node: Option<tree_sitter::Node> = None;
            let mut decorator_node: Option<tree_sitter::Node> = None;
            let mut doc_text: Option<String> = None;

            for capture in m.captures {
                if indices.name == Some(capture.index) {
                    name_text = Some(node_text(&capture.node, source_bytes));
                } else if indices.doc == Some(capture.index) {
                    doc_text = Some(clean_doc_comment(&node_text(&capture.node, source_bytes)));
                } else if indices.reference_call == Some(capture.index) {
                    is_call = true;
                    call_node = Some(capture.node);
                } else if indices.definition_decorator == Some(capture.index) {
                    is_decorator = true;
                    decorator_node = Some(capture.node);
                } else if let Some(kind) = indices.kind_for_capture(capture.index) {
                    def_kind = Some(kind);
                    def_node = Some(capture.node);
                }
            }

            let Some(name) = name_text else { continue };
            if name.is_empty() { continue; }

            if is_decorator {
                let node = decorator_node.unwrap_or(root);
                decorators.push(DecoratorEntry {
                    name,
                    byte_pos: node.start_byte(),
                    span: node_span(&node),
                });
                continue;
            }

            if let (Some(kind), Some(node)) = (def_kind, def_node) {
                let span = node_span(&node);

                // Build signature: first line of definition, truncated
                let sig = {
                    let text = node_text(&node, source_bytes);
                    let first_line = text.lines().next().unwrap_or("");
                    let truncated = if let Some(brace) = first_line.find('{') {
                        first_line[..brace].trim()
                    } else if first_line.len() > 200 {
                        &first_line[..200]
                    } else {
                        first_line
                    };
                    truncated.to_string()
                };

                let gir_node = GirNode {
                    id: SymbolId::new(path, &name, kind, span.start_line),
                    name: name.clone(),
                    kind,
                    file_path: path.to_path_buf(),
                    span,
                    visibility: Visibility::Public,
                    language: self.config.language,
                    signature: Some(sig),
                    complexity: None,
                    confidence: 0.8,
                    doc: doc_text,
                    coverage: None,
                };
                let sym_id = gir_node.id;
                output.add_node(gir_node);
                output.add_edge(file_id, sym_id, GirEdge::new(EdgeKind::Contains));

                definitions.push(DefEntry {
                    id: sym_id,
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                });
            } else if is_call {
                let node = call_node.unwrap_or(root);
                let pos = node.start_position();
                let end = node.end_position();
                calls.push(CallEntry {
                    name,
                    call_start_byte: node.start_byte(),
                    call_start_line: pos.row as u32,
                    call_start_col: pos.column as u32,
                    call_end_line: end.row as u32,
                    call_end_col: end.column as u32,
                });
            }
        }

        // Sort definitions by start_byte for binary search
        definitions.sort_by_key(|d| d.start_byte);

        // Associate decorators with their enclosing definitions.
        // Annotations/attributes are children of the annotated declaration
        // in the AST, so their byte position falls within the definition's range.
        for dec in &decorators {
            let owner_id = find_enclosing_def(&definitions, dec.byte_pos)
                .unwrap_or(file_id);

            let dec_node = GirNode::new(
                dec.name.clone(),
                NodeKind::Decorator,
                path.to_path_buf(),
                dec.span,
                self.config.language,
            );
            let dec_id = dec_node.id;
            output.add_node(dec_node);
            output.add_edge(owner_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
        }

        // Second pass: resolve calls to enclosing functions
        for call in &calls {
            // Find the innermost enclosing definition
            let enclosing_id = find_enclosing_def(&definitions, call.call_start_byte)
                .unwrap_or(file_id);

            let call_span = graphy_core::Span::new(
                call.call_start_line,
                call.call_start_col,
                call.call_end_line,
                call.call_end_col,
            );

            let target = GirNode::new(
                call.name.clone(),
                NodeKind::Function,
                path.to_path_buf(),
                call_span,
                self.config.language,
            );
            let target_id = target.id;
            output.add_node(target);

            output.add_edge(
                enclosing_id,
                target_id,
                GirEdge::new(EdgeKind::Calls)
                    .with_confidence(0.6)
                    .with_metadata(EdgeMetadata::Call { is_dynamic: false }),
            );
        }

        Ok(output)
    }
}

/// Find the innermost definition whose byte range contains the given position.
fn find_enclosing_def(defs: &[DefEntry], byte_pos: usize) -> Option<SymbolId> {
    let mut best: Option<&DefEntry> = None;
    for def in defs {
        if def.start_byte <= byte_pos && byte_pos < def.end_byte {
            match best {
                Some(prev) if (def.end_byte - def.start_byte) < (prev.end_byte - prev.start_byte) => {
                    best = Some(def);
                }
                None => best = Some(def),
                _ => {}
            }
        }
    }
    best.map(|d| d.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags_registry::tags_config_for_language;
    use graphy_core::Language;

    // Dynamic grammar tests: skip gracefully if grammar is not installed.
    // Install with: graphy lang add <name>

    #[test]
    fn parse_go_functions() {
        let Some(config) = tags_config_for_language(Language::Go) else {
            eprintln!("Skipping: Go grammar not installed. Run `graphy lang add go`.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "package main\n\nfunc hello(name string) string {\n    return \"Hello, \" + name\n}\n\nfunc main() {\n    greeting := hello(\"World\")\n    fmt.Println(greeting)\n}\n";
        let output = frontend.parse(Path::new("main.go"), source).unwrap();

        let funcs: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(funcs.iter().any(|f| f.name == "hello" && f.confidence == 0.8));
        assert!(funcs.iter().any(|f| f.name == "main" && f.confidence == 0.8));
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
        let calls: Vec<_> = output.edges.iter().filter(|e| e.2.kind == EdgeKind::Calls).collect();
        assert!(!calls.is_empty());
    }

    #[test]
    fn parse_java_class() {
        let Some(config) = tags_config_for_language(Language::Java) else {
            eprintln!("Skipping: Java grammar not installed. Run `graphy lang add java`.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "public class Calculator {\n    public int add(int a, int b) {\n        return a + b;\n    }\n    public int multiply(int a, int b) {\n        return a * b;\n    }\n}\n";
        let output = frontend.parse(Path::new("Calculator.java"), source).unwrap();

        let classes: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Calculator");
        let methods: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn parse_php_wordpress() {
        let Some(config) = tags_config_for_language(Language::Php) else {
            eprintln!("Skipping: PHP grammar not installed. Run `graphy lang add php`.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "<?php\nfunction register_post_type($name, $args) {\n    return wp_insert_post($args);\n}\n\nclass CustomPlugin {\n    public function activate() {\n        register_post_type('custom', array());\n    }\n}\n?>";
        let output = frontend.parse(Path::new("plugin.php"), source).unwrap();

        let funcs: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Function && n.confidence == 0.8).collect();
        assert!(funcs.iter().any(|f| f.name == "register_post_type"));
        let classes: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert!(classes.iter().any(|c| c.name == "CustomPlugin"));
    }

    #[test]
    fn parse_c_functions() {
        let Some(config) = tags_config_for_language(Language::C) else {
            eprintln!("Skipping: C grammar not installed. Run `graphy lang add c`.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "struct Point {\n    int x;\n    int y;\n};\n\nint distance(struct Point a, struct Point b) {\n    return sqrt(pow(b.x - a.x, 2) + pow(b.y - a.y, 2));\n}\n";
        let output = frontend.parse(Path::new("geo.c"), source).unwrap();

        let funcs: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Function && n.confidence == 0.8).collect();
        assert!(funcs.iter().any(|f| f.name == "distance"));
        let classes: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert!(classes.iter().any(|c| c.name == "Point"));
    }

    #[test]
    fn parse_ruby_class() {
        let Some(config) = tags_config_for_language(Language::Ruby) else {
            eprintln!("Skipping: Ruby grammar not installed. Run `graphy lang add ruby`.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "class Dog\n  def initialize(name)\n    @name = name\n  end\n\n  def bark\n    puts \"Woof!\"\n  end\nend\n";
        let output = frontend.parse(Path::new("dog.rb"), source).unwrap();

        let classes: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Dog");
        let methods: Vec<_> = output.nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "initialize"));
        assert!(methods.iter().any(|m| m.name == "bark"));
    }

    #[test]
    fn calls_attributed_to_enclosing_function() {
        let Some(config) = tags_config_for_language(Language::Go) else {
            eprintln!("Skipping: Go grammar not installed.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let source = "package main\n\nfunc outer() {\n    inner()\n}\n\nfunc inner() {}\n";
        let output = frontend.parse(Path::new("main.go"), source).unwrap();

        let outer = output.nodes.iter().find(|n| n.name == "outer" && n.confidence == 0.8).unwrap();
        let call_edge = output.edges.iter().find(|e| e.2.kind == EdgeKind::Calls).unwrap();
        assert_eq!(call_edge.0, outer.id);
    }

    #[test]
    fn file_node_always_present() {
        let Some(config) = tags_config_for_language(Language::Go) else {
            eprintln!("Skipping: Go grammar not installed.");
            return;
        };
        let frontend = TagsFrontend::new(config);
        let output = frontend.parse(Path::new("empty.go"), "").unwrap();
        assert_eq!(output.nodes.len(), 1);
        assert_eq!(output.nodes[0].kind, NodeKind::File);
    }

    #[test]
    fn find_enclosing_def_edge_cases() {
        // Empty definitions list: should return None
        assert!(find_enclosing_def(&[], 50).is_none());

        // Single definition, position outside range
        let defs = vec![DefEntry {
            id: SymbolId::new(
                std::path::Path::new("test.go"),
                "func1",
                NodeKind::Function,
                0,
            ),
            start_byte: 10,
            end_byte: 50,
        }];
        assert!(find_enclosing_def(&defs, 5).is_none());  // before range
        assert!(find_enclosing_def(&defs, 50).is_none()); // at end_byte (exclusive)
        assert!(find_enclosing_def(&defs, 55).is_none()); // after range
        assert!(find_enclosing_def(&defs, 10).is_some()); // at start_byte (inclusive)
        assert!(find_enclosing_def(&defs, 30).is_some()); // inside range

        // Nested definitions: should pick the innermost (smallest range)
        let outer_id = SymbolId::new(
            std::path::Path::new("test.go"), "outer", NodeKind::Function, 0,
        );
        let inner_id = SymbolId::new(
            std::path::Path::new("test.go"), "inner", NodeKind::Function, 5,
        );
        let nested_defs = vec![
            DefEntry { id: outer_id, start_byte: 0, end_byte: 100 },
            DefEntry { id: inner_id, start_byte: 20, end_byte: 60 },
        ];
        // Position inside inner: should return inner (smaller range)
        assert_eq!(find_enclosing_def(&nested_defs, 30), Some(inner_id));
        // Position inside outer but outside inner: should return outer
        assert_eq!(find_enclosing_def(&nested_defs, 10), Some(outer_id));
        assert_eq!(find_enclosing_def(&nested_defs, 80), Some(outer_id));
    }
}
