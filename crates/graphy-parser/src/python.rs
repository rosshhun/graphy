use std::path::Path;

use anyhow::{Context, Result};
use graphy_core::{
    EdgeKind, EdgeMetadata, GirEdge, GirNode, Language, NodeKind, ParseOutput, SymbolId,
    Visibility,
};
use tree_sitter::{Node, Parser};

use crate::frontend::LanguageFrontend;
use crate::helpers::{is_noise_method_call, node_span, node_text};

pub struct PythonFrontend;

impl PythonFrontend {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageFrontend for PythonFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput> {
        // tree-sitter Parser isn't Sync, so we create a new one per parse.
        // This is cheap — the grammar is shared.
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .context("Failed to set Python language")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse returned None")?;

        let root = tree.root_node();
        let mut output = ParseOutput::new();
        let source_bytes = source.as_bytes();

        // Create the file/module node
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
            language: Language::Python,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        };
        let file_id = file_node.id;
        output.add_node(file_node);

        // Walk top-level children
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            extract_node(&child, source_bytes, path, file_id, &mut output);
        }

        Ok(output)
    }
}

fn extract_node(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    match node.kind() {
        "function_definition" => {
            extract_function(node, source, path, parent_id, output, false);
        }
        "class_definition" => {
            extract_class(node, source, path, parent_id, output);
        }
        "decorated_definition" => {
            extract_decorated(node, source, path, parent_id, output);
        }
        "import_statement" => {
            extract_import(node, source, path, parent_id, output);
        }
        "import_from_statement" => {
            extract_import_from(node, source, path, parent_id, output);
        }
        "expression_statement" => {
            // Check for module-level assignments like `__all__ = [...]`
            if let Some(child) = node.child(0) {
                if child.kind() == "assignment" {
                    extract_assignment(&child, source, path, parent_id, output);
                }
            }
        }
        "assignment" => {
            extract_assignment(node, source, path, parent_id, output);
        }
        _ => {}
    }
}

fn extract_function(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    is_method: bool,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let kind = if is_method {
        if name == "__init__" {
            NodeKind::Constructor
        } else {
            NodeKind::Method
        }
    } else {
        NodeKind::Function
    };

    let visibility = python_visibility(&name);

    // Build signature
    let sig = build_function_signature(node, source, &name);

    // Extract docstring
    let doc = extract_docstring(node, source);

    let func_node = GirNode {
        id: SymbolId::new(path, &name, kind, span.start_line),
        name: name.clone(),
        kind,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Python,
        signature: Some(sig),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let func_id = func_node.id;
    output.add_node(func_node);

    // CONTAINS edge: parent -> function
    output.add_edge(parent_id, func_id, GirEdge::new(EdgeKind::Contains));

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_parameters(&params, source, path, func_id, output);
    }

    // Extract return type annotation
    if let Some(ret) = node.child_by_field_name("return_type") {
        let type_name = node_text(&ret, source);
        let type_node = GirNode::new(
            type_name,
            NodeKind::TypeAlias,
            path.to_path_buf(),
            node_span(&ret),
            Language::Python,
        );
        let type_id = type_node.id;
        output.add_node(type_node);
        output.add_edge(func_id, type_id, GirEdge::new(EdgeKind::ReturnsType));
    }

    // Walk function body for calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, path, func_id, output);
    }
}

fn extract_class(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let doc = extract_docstring(node, source);

    let class_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Class, span.start_line),
        name: name.clone(),
        kind: NodeKind::Class,
        file_path: path.to_path_buf(),
        span,
        visibility: python_visibility(&name),
        language: Language::Python,
        signature: Some(format!("class {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let class_id = class_node.id;
    output.add_node(class_node);

    // CONTAINS edge: parent -> class
    output.add_edge(parent_id, class_id, GirEdge::new(EdgeKind::Contains));

    // Extract base classes (inheritance)
    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        let mut cursor = superclasses.walk();
        for arg in superclasses.children(&mut cursor) {
            if arg.kind() == "identifier" || arg.kind() == "attribute" {
                let base_name = node_text(&arg, source);
                let base_node = GirNode::new(
                    base_name,
                    NodeKind::Class,
                    path.to_path_buf(),
                    node_span(&arg),
                    Language::Python,
                );
                let base_id = base_node.id;
                output.add_node(base_node);
                output.add_edge(
                    class_id,
                    base_id,
                    GirEdge::new(EdgeKind::Inherits)
                        .with_metadata(EdgeMetadata::Inheritance { depth: 1 }),
                );
            }
        }
    }

    // Walk class body for methods and class-level assignments
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    extract_function(&child, source, path, class_id, output, true);
                }
                "decorated_definition" => {
                    extract_decorated(&child, source, path, class_id, output);
                }
                "expression_statement" => {
                    if let Some(assign) = child.child(0) {
                        if assign.kind() == "assignment" {
                            extract_class_field(&assign, source, path, class_id, output);
                        }
                    }
                }
                "assignment" => {
                    extract_class_field(&child, source, path, class_id, output);
                }
                _ => {}
            }
        }
    }
}

fn extract_decorated(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    // Find the actual definition (last child that isn't a decorator)
    let child_count = node.child_count();
    if child_count == 0 {
        return;
    }

    // Collect decorators
    let mut decorators = Vec::new();
    let mut definition = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let dec_text = node_text(&child, source);
            // Strip leading @
            let dec_name = dec_text.trim_start_matches('@').trim().to_string();
            decorators.push((dec_name, node_span(&child)));
        } else {
            definition = Some(child);
        }
    }

    let Some(def_node) = definition else {
        return;
    };

    // Check if this is a method inside a class (parent has class kind)
    let is_method = parent_id != SymbolId::new(path, path.to_string_lossy().as_ref(), NodeKind::File, 0)
        && def_node.kind() == "function_definition";

    match def_node.kind() {
        "function_definition" => {
            extract_function(&def_node, source, path, parent_id, output, is_method);

            // Add decorator edges
            if let Some(name_node) = def_node.child_by_field_name("name") {
                let func_name = node_text(&name_node, source);
                let func_span = node_span(&def_node);
                let func_kind = if is_method {
                    if func_name == "__init__" {
                        NodeKind::Constructor
                    } else {
                        NodeKind::Method
                    }
                } else {
                    NodeKind::Function
                };
                let func_id = SymbolId::new(path, &func_name, func_kind, func_span.start_line);

                for (dec_name, dec_span) in &decorators {
                    let dec_node = GirNode::new(
                        dec_name.clone(),
                        NodeKind::Decorator,
                        path.to_path_buf(),
                        *dec_span,
                        Language::Python,
                    );
                    let dec_id = dec_node.id;
                    output.add_node(dec_node);
                    output.add_edge(func_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
                }
            }
        }
        "class_definition" => {
            extract_class(&def_node, source, path, parent_id, output);

            if let Some(name_node) = def_node.child_by_field_name("name") {
                let class_name = node_text(&name_node, source);
                let class_span = node_span(&def_node);
                let class_id =
                    SymbolId::new(path, &class_name, NodeKind::Class, class_span.start_line);

                for (dec_name, dec_span) in &decorators {
                    let dec_node = GirNode::new(
                        dec_name.clone(),
                        NodeKind::Decorator,
                        path.to_path_buf(),
                        *dec_span,
                        Language::Python,
                    );
                    let dec_id = dec_node.id;
                    output.add_node(dec_node);
                    output.add_edge(class_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
                }
            }
        }
        _ => {}
    }
}

fn extract_import(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    // `import foo` or `import foo.bar` or `import foo as bar`
    let text = node_text(node, source);
    let span = node_span(node);

    let import_node = GirNode {
        id: SymbolId::new(path, &text, NodeKind::Import, span.start_line),
        name: text.clone(),
        kind: NodeKind::Import,
        file_path: path.to_path_buf(),
        span,
        visibility: Visibility::Internal,
        language: Language::Python,
        signature: Some(text),
        complexity: None,
        confidence: 1.0,
        doc: None,
        coverage: None,
    };
    let import_id = import_node.id;
    output.add_node(import_node);
    output.add_edge(parent_id, import_id, GirEdge::new(EdgeKind::Contains));
}

fn extract_import_from(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    let text = node_text(node, source);
    let span = node_span(node);

    // Extract the module name and imported items
    let module_name = node
        .child_by_field_name("module_name")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
            // Skip the module name (first dotted_name is the module)
            let item_text = node_text(&child, source);
            if item_text != module_name {
                items.push(item_text);
            }
        }
    }

    let import_node = GirNode {
        id: SymbolId::new(path, &text, NodeKind::Import, span.start_line),
        name: module_name.clone(),
        kind: NodeKind::Import,
        file_path: path.to_path_buf(),
        span,
        visibility: Visibility::Internal,
        language: Language::Python,
        signature: Some(text),
        complexity: None,
        confidence: 1.0,
        doc: None,
        coverage: None,
    };
    let import_id = import_node.id;
    output.add_node(import_node);
    output.add_edge(
        parent_id,
        import_id,
        GirEdge::new(EdgeKind::ImportsFrom).with_metadata(EdgeMetadata::Import {
            alias: None,
            items,
        }),
    );
}

fn extract_assignment(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };

    // Only handle simple name = value assignments
    if left.kind() != "identifier" {
        return;
    }

    let name = node_text(&left, source);
    let span = node_span(node);

    // Determine if it's a constant (ALL_CAPS convention)
    let is_constant = name.chars().all(|c| c.is_uppercase() || c == '_') && !name.is_empty();
    let kind = if is_constant {
        NodeKind::Constant
    } else {
        NodeKind::Variable
    };

    let var_node = GirNode::new(
        name,
        kind,
        path.to_path_buf(),
        span,
        Language::Python,
    );
    let var_id = var_node.id;
    output.add_node(var_node);
    output.add_edge(parent_id, var_id, GirEdge::new(EdgeKind::Contains));
}

fn extract_class_field(
    node: &Node,
    source: &[u8],
    path: &Path,
    class_id: SymbolId,
    output: &mut ParseOutput,
) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };

    if left.kind() != "identifier" {
        return;
    }

    let name = node_text(&left, source);
    let span = node_span(node);

    let field_node = GirNode::new(
        name,
        NodeKind::Field,
        path.to_path_buf(),
        span,
        Language::Python,
    );
    let field_id = field_node.id;
    output.add_node(field_node);
    output.add_edge(class_id, field_id, GirEdge::new(EdgeKind::Contains));
}

fn extract_parameters(
    params_node: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
) {
    let mut cursor = params_node.walk();
    for param in params_node.children(&mut cursor) {
        let name = match param.kind() {
            "identifier" => node_text(&param, source),
            "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                param
                    .child_by_field_name("name")
                    .or_else(|| param.child(0))
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                // *args, **kwargs
                param
                    .child(0)
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            _ => continue,
        };

        if name.is_empty() || name == "," || name == "(" || name == ")" {
            continue;
        }

        let param_node = GirNode::new(
            name,
            NodeKind::Parameter,
            path.to_path_buf(),
            node_span(&param),
            Language::Python,
        );
        let param_id = param_node.id;
        output.add_node(param_node);
        output.add_edge(func_id, param_id, GirEdge::new(EdgeKind::Contains));

        // Extract type annotation if present
        if let Some(type_node) = param.child_by_field_name("type") {
            let type_name = node_text(&type_node, source);
            let tn = GirNode::new(
                type_name,
                NodeKind::TypeAlias,
                path.to_path_buf(),
                node_span(&type_node),
                Language::Python,
            );
            let type_id = tn.id;
            output.add_node(tn);
            output.add_edge(param_id, type_id, GirEdge::new(EdgeKind::ParamType));
        }
    }
}

/// Walk a function body and extract call expressions.
fn extract_calls_from_body(
    body: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
) {
    let mut stack = vec![*body];
    while let Some(node) = stack.pop() {
        if node.kind() == "call" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let call_name = node_text(&func_node, source);

                // Skip builtins and method chains on local variables
                if !is_noise_builtin(&call_name) && !is_noise_method_call(&call_name) {
                    let call_target = GirNode::new(
                        call_name.clone(),
                        NodeKind::Function,
                        path.to_path_buf(),
                        node_span(&func_node),
                        Language::Python,
                    );
                    let target_id = call_target.id;
                    output.add_node(call_target);

                    let confidence = if call_name.contains('.') { 0.7 } else { 0.9 };
                    output.add_edge(
                        func_id,
                        target_id,
                        GirEdge::new(EdgeKind::Calls)
                            .with_confidence(confidence)
                            .with_metadata(EdgeMetadata::Call {
                                is_dynamic: call_name.contains('.'),
                            }),
                    );
                }
            }
        }

        // Don't recurse into nested function/class definitions
        let is_nested = node.kind() == "function_definition" || node.kind() == "class_definition";
        if !is_nested || node == *body {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && name.ends_with("__") {
        Visibility::Public // dunder methods are public protocol
    } else if name.starts_with("__") {
        Visibility::Private
    } else if name.starts_with('_') {
        Visibility::Internal
    } else {
        Visibility::Public
    }
}

fn is_noise_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "range"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "type"
            | "isinstance"
            | "issubclass"
            | "hasattr"
            | "getattr"
            | "setattr"
            | "super"
            | "repr"
            | "hash"
            | "id"
            | "input"
            | "open"
            | "next"
            | "iter"
            | "abs"
            | "min"
            | "max"
            | "sum"
            | "any"
            | "all"
    )
}

fn build_function_signature(node: &Node, source: &[u8], name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, source))
        .unwrap_or_else(|| "()".to_string());

    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(" -> {}", node_text(&r, source)))
        .unwrap_or_default();

    format!("def {name}{params}{ret}")
}

fn extract_docstring(node: &Node, source: &[u8]) -> Option<String> {
    // Docstring is the first expression_statement in the body that contains a string
    let body = node.child_by_field_name("body")?;
    let first = body.child(0)?;
    if first.kind() == "expression_statement" {
        let expr = first.child(0)?;
        if expr.kind() == "string" || expr.kind() == "concatenated_string" {
            let text = node_text(&expr, source);
            // Strip triple quotes
            let trimmed = text
                .trim_start_matches("\"\"\"")
                .trim_start_matches("'''")
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim();
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::NodeKind;

    #[test]
    fn parse_simple_function() {
        let source = r#"
def hello(name: str) -> str:
    """Greet someone."""
    return f"Hello, {name}!"
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "hello");
        assert_eq!(funcs[0].doc.as_deref(), Some("Greet someone."));
        assert!(funcs[0].signature.as_ref().unwrap().contains("-> str"));
    }

    #[test]
    fn parse_class_with_methods() {
        let source = r#"
class Dog(Animal):
    name = "default"

    def __init__(self, name: str):
        self.name = name

    def bark(self) -> str:
        return "Woof!"
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();

        let classes: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert!(classes.iter().any(|c| c.name == "Dog"));

        let constructors: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constructor)
            .collect();
        assert_eq!(constructors.len(), 1);

        let methods: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "bark");
    }

    #[test]
    fn parse_imports() {
        let source = r#"
import os
from pathlib import Path
from . import utils
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();

        let imports: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn parse_decorated_function() {
        let source = r#"
@app.route("/hello")
def hello():
    return "Hello!"
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();

        let decorators: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Decorator)
            .collect();
        assert!(!decorators.is_empty());

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
    }

    #[test]
    fn visibility_detection() {
        assert_eq!(python_visibility("public_func"), Visibility::Public);
        assert_eq!(python_visibility("_internal"), Visibility::Internal);
        assert_eq!(python_visibility("__private"), Visibility::Private);
        assert_eq!(python_visibility("__init__"), Visibility::Public);
    }

    // ── Edge case tests ───────────────────────────────────

    #[test]
    fn parse_empty_file() {
        let output = PythonFrontend::new()
            .parse(Path::new("empty.py"), "")
            .unwrap();
        // Should have a File node and nothing else
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert!(output.nodes.iter().filter(|n| n.kind != NodeKind::File).count() == 0
            || output.nodes.len() == 1);
    }

    #[test]
    fn parse_comments_only() {
        let source = "# This is a comment\n# Another comment\n";
        let output = PythonFrontend::new()
            .parse(Path::new("comments.py"), source)
            .unwrap();
        // Only the file node, no functions/classes
        let non_file: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function || n.kind == NodeKind::Class)
            .collect();
        assert!(non_file.is_empty());
    }

    #[test]
    fn parse_async_function() {
        let source = r#"
async def fetch_data(url: str) -> dict:
    """Fetch data from URL."""
    pass
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "fetch_data");
    }

    #[test]
    fn parse_nested_class() {
        let source = r#"
class Outer:
    class Inner:
        def inner_method(self):
            pass

    def outer_method(self):
        pass
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        let classes: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        // Parser processes nested classes — at least the Outer class is found
        assert!(classes.len() >= 1);
        assert!(classes.iter().any(|c| c.name == "Outer"));
    }

    #[test]
    fn parse_multiple_decorators() {
        let source = r#"
@staticmethod
@cache
def cached_static():
    pass
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        let decorators: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Decorator)
            .collect();
        assert!(decorators.len() >= 2);
    }

    #[test]
    fn parse_global_variable() {
        let source = r#"
MAX_RETRIES = 3
_internal_flag = True
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        let vars: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Variable || n.kind == NodeKind::Constant)
            .collect();
        assert!(vars.len() >= 1);
    }

    #[test]
    fn parse_function_with_call_expressions() {
        let source = r#"
def foo():
    bar()
    baz.quux()
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        // Should have at least the function + call target nodes
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(funcs.len() >= 1);
        // Should have Calls edges
        let calls: Vec<_> = output.edges.iter()
            .filter(|e| e.2.kind == EdgeKind::Calls)
            .collect();
        assert!(calls.len() >= 1);
    }

    #[test]
    fn parse_star_import() {
        let source = "from os.path import *\n";
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();
        let imports: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();
        assert_eq!(imports.len(), 1);
    }

    #[test]
    fn parse_stacked_decorators_on_async_method() {
        let source = r#"
class Service:
    @staticmethod
    @cache
    @log_calls
    async def fetch_all(url: str) -> list:
        """Fetch all items."""
        pass
"#;
        let output = PythonFrontend::new()
            .parse(Path::new("test.py"), source)
            .unwrap();

        // The async method should be extracted
        let methods: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "fetch_all");

        // All three stacked decorators should be present
        let decorators: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Decorator)
            .collect();
        assert!(decorators.len() >= 3, "Expected 3 decorators, got {}", decorators.len());
        let dec_names: Vec<&str> = decorators.iter().map(|d| d.name.as_str()).collect();
        assert!(dec_names.contains(&"staticmethod"));
        assert!(dec_names.contains(&"cache"));
        assert!(dec_names.contains(&"log_calls"));

        // Each decorator should have an AnnotatedWith edge to the method
        let annotated_edges: Vec<_> = output.edges.iter()
            .filter(|e| e.2.kind == EdgeKind::AnnotatedWith)
            .collect();
        assert!(annotated_edges.len() >= 3);
    }
}
