use std::path::Path;

use anyhow::{Context, Result};
use graphy_core::{
    EdgeKind, EdgeMetadata, GirEdge, GirNode, Language, NodeKind, ParseOutput, SymbolId,
    Visibility,
};
use tree_sitter::{Node, Parser};

use crate::frontend::LanguageFrontend;
use crate::helpers::{is_noise_method_call, node_span, node_text};

/// Frontend for TypeScript (.ts, .tsx) and JavaScript (.js, .jsx, .mjs, .cjs) files.
pub struct TypeScriptFrontend;

impl TypeScriptFrontend {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageFrontend for TypeScriptFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput> {
        let mut parser = Parser::new();

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Pick the right tree-sitter grammar for the file extension
        match ext {
            "ts" => {
                parser
                    .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                    .context("Failed to set TypeScript language")?;
            }
            "tsx" => {
                parser
                    .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
                    .context("Failed to set TSX language")?;
            }
            // JS, JSX, MJS, CJS all use the JavaScript grammar
            _ => {
                parser
                    .set_language(&tree_sitter_javascript::LANGUAGE.into())
                    .context("Failed to set JavaScript language")?;
            }
        }

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse returned None")?;

        let root = tree.root_node();
        let mut output = ParseOutput::new();
        let source_bytes = source.as_bytes();

        let language = match ext {
            "ts" | "tsx" => Language::TypeScript,
            _ => Language::JavaScript,
        };

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
            language,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        };
        let file_id = file_node.id;
        output.add_node(file_node);

        // Track which top-level names are exported (for visibility determination)
        let exported_names = collect_exported_names(&root, source_bytes);

        // Walk top-level children
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            extract_node(
                &child,
                source_bytes,
                path,
                file_id,
                &mut output,
                language,
                &exported_names,
            );
        }

        Ok(output)
    }
}

/// Collect names that are explicitly exported at the top level.
fn collect_exported_names(root: &Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "export_statement" {
            // `export function foo`, `export class Foo`, `export default ...`
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                match inner.kind() {
                    "function_declaration" | "class_declaration"
                    | "interface_declaration" | "enum_declaration"
                    | "type_alias_declaration" => {
                        if let Some(name_node) = inner.child_by_field_name("name") {
                            names.push(node_text(&name_node, source));
                        }
                    }
                    "lexical_declaration" | "variable_declaration" => {
                        collect_variable_names(&inner, source, &mut names);
                    }
                    _ => {}
                }
            }
            // `export { x, y }` — named exports
            let mut inner_cursor2 = child.walk();
            for inner in child.children(&mut inner_cursor2) {
                if inner.kind() == "export_clause" {
                    let mut clause_cursor = inner.walk();
                    for spec in inner.children(&mut clause_cursor) {
                        if spec.kind() == "export_specifier" {
                            if let Some(name_node) = spec.child_by_field_name("name") {
                                names.push(node_text(&name_node, source));
                            }
                        }
                    }
                }
            }
        }
    }
    names
}

fn collect_variable_names(node: &Node, source: &[u8], names: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                names.push(node_text(&name_node, source));
            }
        }
    }
}

fn extract_node(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    match node.kind() {
        "function_declaration" => {
            extract_function(node, source, path, parent_id, output, language, false, exported_names);
        }
        "generator_function_declaration" => {
            extract_function(node, source, path, parent_id, output, language, false, exported_names);
        }
        "class_declaration" => {
            extract_class(node, source, path, parent_id, output, language, exported_names);
        }
        "interface_declaration" => {
            extract_interface(node, source, path, parent_id, output, language, exported_names);
        }
        "type_alias_declaration" => {
            extract_type_alias(node, source, path, parent_id, output, language, exported_names);
        }
        "enum_declaration" => {
            extract_enum(node, source, path, parent_id, output, language, exported_names);
        }
        "lexical_declaration" | "variable_declaration" => {
            extract_variable_declaration(node, source, path, parent_id, output, language, exported_names);
        }
        "import_statement" => {
            extract_import(node, source, path, parent_id, output, language);
        }
        "export_statement" => {
            extract_export(node, source, path, parent_id, output, language, exported_names);
        }
        "expression_statement" => {
            // Handle `module.exports = ...` or top-level expressions with require()
            extract_expression_statement(node, source, path, parent_id, output, language);
        }
        _ => {}
    }
}

// ── Functions ───────────────────────────────────────────────

fn extract_function(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    is_method: bool,
    exported_names: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let kind = if is_method {
        if name == "constructor" {
            NodeKind::Constructor
        } else {
            NodeKind::Method
        }
    } else {
        NodeKind::Function
    };

    let visibility = if exported_names.contains(&name) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let sig = build_function_signature(node, source, &name);
    let doc = extract_jsdoc(node, source);

    let func_node = GirNode {
        id: SymbolId::new(path, &name, kind, span.start_line),
        name: name.clone(),
        kind,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(sig),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let func_id = func_node.id;
    output.add_node(func_node);
    output.add_edge(parent_id, func_id, GirEdge::new(EdgeKind::Contains));

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_parameters(&params, source, path, func_id, output, language);
    }

    // Extract return type annotation
    if let Some(ret) = node.child_by_field_name("return_type") {
        extract_return_type(&ret, source, path, func_id, output, language);
    }

    // Walk body for calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, path, func_id, output, language);
    }

    // Extract decorators
    extract_decorators(node, source, path, func_id, output, language);
}

fn extract_arrow_function(
    node: &Node,
    name: &str,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let span = node_span(node);
    let kind = NodeKind::Function;

    let visibility = if exported_names.contains(&name.to_string()) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let sig = build_arrow_signature(node, source, name);
    let doc = extract_jsdoc(node, source);

    let func_node = GirNode {
        id: SymbolId::new(path, name, kind, span.start_line),
        name: name.to_string(),
        kind,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(sig),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let func_id = func_node.id;
    output.add_node(func_node);
    output.add_edge(parent_id, func_id, GirEdge::new(EdgeKind::Contains));

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_parameters(&params, source, path, func_id, output, language);
    } else if let Some(param) = node.child_by_field_name("parameter") {
        // Single parameter arrow function: x => x + 1
        let param_name = node_text(&param, source);
        if !param_name.is_empty() {
            let param_node = GirNode::new(
                param_name,
                NodeKind::Parameter,
                path.to_path_buf(),
                node_span(&param),
                language,
            );
            let param_id = param_node.id;
            output.add_node(param_node);
            output.add_edge(func_id, param_id, GirEdge::new(EdgeKind::Contains));
        }
    }

    // Extract return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        extract_return_type(&ret, source, path, func_id, output, language);
    }

    // Walk body for calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, path, func_id, output, language);
    }
}

// ── Classes ─────────────────────────────────────────────────

fn extract_class(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let doc = extract_jsdoc(node, source);

    let visibility = if exported_names.contains(&name) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let class_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Class, span.start_line),
        name: name.clone(),
        kind: NodeKind::Class,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(format!("class {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let class_id = class_node.id;
    output.add_node(class_node);
    output.add_edge(parent_id, class_id, GirEdge::new(EdgeKind::Contains));

    // Extract superclass (extends)
    if let Some(heritage) = node.child_by_field_name("heritage") {
        // The heritage clause node may directly contain the superclass name
        let heritage_text = node_text(&heritage, source);
        if !heritage_text.is_empty() {
            let base_node = GirNode::new(
                heritage_text.clone(),
                NodeKind::Class,
                path.to_path_buf(),
                node_span(&heritage),
                language,
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

    // Also check class_heritage for extends/implements via walking children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            let mut heritage_cursor = child.walk();
            for heritage_child in child.children(&mut heritage_cursor) {
                if heritage_child.kind() == "extends_clause" {
                    if let Some(value) = heritage_child.child(1) {
                        let base_name = node_text(&value, source);
                        let base_node = GirNode::new(
                            base_name,
                            NodeKind::Class,
                            path.to_path_buf(),
                            node_span(&value),
                            language,
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
                if heritage_child.kind() == "implements_clause" {
                    let mut impl_cursor = heritage_child.walk();
                    for impl_child in heritage_child.children(&mut impl_cursor) {
                        if impl_child.kind() == "type_identifier"
                            || impl_child.kind() == "generic_type"
                        {
                            let iface_name = node_text(&impl_child, source);
                            let iface_node = GirNode::new(
                                iface_name,
                                NodeKind::Interface,
                                path.to_path_buf(),
                                node_span(&impl_child),
                                language,
                            );
                            let iface_id = iface_node.id;
                            output.add_node(iface_node);
                            output.add_edge(
                                class_id,
                                iface_id,
                                GirEdge::new(EdgeKind::Implements),
                            );
                        }
                    }
                }
            }
        }
    }

    // Walk class body for methods, properties, fields
    if let Some(body) = node.child_by_field_name("body") {
        let mut body_cursor = body.walk();
        for child in body.children(&mut body_cursor) {
            match child.kind() {
                "method_definition" => {
                    extract_method(&child, source, path, class_id, output, language);
                }
                "public_field_definition" | "field_definition" => {
                    extract_class_field(&child, source, path, class_id, output, language);
                }
                _ => {}
            }
        }
    }

    // Extract decorators on the class
    extract_decorators(node, source, path, class_id, output, language);
}

fn extract_method(
    node: &Node,
    source: &[u8],
    path: &Path,
    class_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let kind = if name == "constructor" {
        NodeKind::Constructor
    } else {
        NodeKind::Method
    };

    let visibility = method_visibility(node, source);
    let sig = build_function_signature(node, source, &name);
    let doc = extract_jsdoc(node, source);

    let method_node = GirNode {
        id: SymbolId::new(path, &name, kind, span.start_line),
        name: name.clone(),
        kind,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(sig),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let method_id = method_node.id;
    output.add_node(method_node);
    output.add_edge(class_id, method_id, GirEdge::new(EdgeKind::Contains));

    // Parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_parameters(&params, source, path, method_id, output, language);
    }

    // Return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        extract_return_type(&ret, source, path, method_id, output, language);
    }

    // Body calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, path, method_id, output, language);
    }

    // Decorators
    extract_decorators(node, source, path, method_id, output, language);
}

fn extract_class_field(
    node: &Node,
    source: &[u8],
    path: &Path,
    class_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        // fallback: try first named child
        if let Some(first) = node.named_child(0) {
            let name = node_text(&first, source);
            if !name.is_empty() {
                let field_node = GirNode::new(
                    name,
                    NodeKind::Field,
                    path.to_path_buf(),
                    node_span(node),
                    language,
                );
                let field_id = field_node.id;
                output.add_node(field_node);
                output.add_edge(class_id, field_id, GirEdge::new(EdgeKind::Contains));
            }
        }
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let field_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Field, span.start_line),
        name,
        kind: NodeKind::Field,
        file_path: path.to_path_buf(),
        span,
        visibility: method_visibility(node, source),
        language,
        signature: None,
        complexity: None,
        confidence: 1.0,
        doc: None,
        coverage: None,
    };
    let field_id = field_node.id;
    output.add_node(field_node);
    output.add_edge(class_id, field_id, GirEdge::new(EdgeKind::Contains));

    // Extract field type annotation
    if let Some(type_ann) = node.child_by_field_name("type") {
        let type_name = node_text(&type_ann, source);
        let type_node = GirNode::new(
            type_name,
            NodeKind::TypeAlias,
            path.to_path_buf(),
            node_span(&type_ann),
            language,
        );
        let type_id = type_node.id;
        output.add_node(type_node);
        output.add_edge(field_id, type_id, GirEdge::new(EdgeKind::FieldType));
    }
}

// ── Interfaces ──────────────────────────────────────────────

fn extract_interface(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let doc = extract_jsdoc(node, source);

    let visibility = if exported_names.contains(&name) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let iface_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Interface, span.start_line),
        name: name.clone(),
        kind: NodeKind::Interface,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(format!("interface {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let iface_id = iface_node.id;
    output.add_node(iface_node);
    output.add_edge(parent_id, iface_id, GirEdge::new(EdgeKind::Contains));

    // Walk interface body for property signatures and method signatures
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "property_signature" | "public_field_definition" => {
                    if let Some(pname) = child.child_by_field_name("name") {
                        let prop_name = node_text(&pname, source);
                        let prop_node = GirNode::new(
                            prop_name,
                            NodeKind::Property,
                            path.to_path_buf(),
                            node_span(&child),
                            language,
                        );
                        let prop_id = prop_node.id;
                        output.add_node(prop_node);
                        output.add_edge(iface_id, prop_id, GirEdge::new(EdgeKind::Contains));
                    }
                }
                "method_signature" => {
                    if let Some(mname) = child.child_by_field_name("name") {
                        let method_name = node_text(&mname, source);
                        let method_node = GirNode::new(
                            method_name,
                            NodeKind::Method,
                            path.to_path_buf(),
                            node_span(&child),
                            language,
                        );
                        let method_id = method_node.id;
                        output.add_node(method_node);
                        output.add_edge(iface_id, method_id, GirEdge::new(EdgeKind::Contains));
                    }
                }
                _ => {}
            }
        }
    }

    // extends clause for interfaces
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_type_clause" || child.kind() == "extends_clause" {
            let mut ext_cursor = child.walk();
            for ext_child in child.children(&mut ext_cursor) {
                if ext_child.kind() == "type_identifier" || ext_child.kind() == "generic_type" {
                    let base_name = node_text(&ext_child, source);
                    let base_node = GirNode::new(
                        base_name,
                        NodeKind::Interface,
                        path.to_path_buf(),
                        node_span(&ext_child),
                        language,
                    );
                    let base_id = base_node.id;
                    output.add_node(base_node);
                    output.add_edge(
                        iface_id,
                        base_id,
                        GirEdge::new(EdgeKind::Inherits)
                            .with_metadata(EdgeMetadata::Inheritance { depth: 1 }),
                    );
                }
            }
        }
    }
}

// ── Type Aliases ────────────────────────────────────────────

fn extract_type_alias(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let full_text = node_text(node, source);

    let visibility = if exported_names.contains(&name) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let type_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::TypeAlias, span.start_line),
        name: name.clone(),
        kind: NodeKind::TypeAlias,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(full_text),
        complexity: None,
        confidence: 1.0,
        doc: extract_jsdoc(node, source),
        coverage: None,
    };
    let type_id = type_node.id;
    output.add_node(type_node);
    output.add_edge(parent_id, type_id, GirEdge::new(EdgeKind::Contains));
}

// ── Enums ───────────────────────────────────────────────────

fn extract_enum(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let visibility = if exported_names.contains(&name) {
        Visibility::Public
    } else {
        Visibility::Internal
    };

    let enum_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Enum, span.start_line),
        name: name.clone(),
        kind: NodeKind::Enum,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language,
        signature: Some(format!("enum {name}")),
        complexity: None,
        confidence: 1.0,
        doc: extract_jsdoc(node, source),
        coverage: None,
    };
    let enum_id = enum_node.id;
    output.add_node(enum_node);
    output.add_edge(parent_id, enum_id, GirEdge::new(EdgeKind::Contains));

    // Extract enum members
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_member" || child.kind() == "property_identifier" {
                if let Some(member_name_node) = child.child_by_field_name("name") {
                    let member_name = node_text(&member_name_node, source);
                    let variant_node = GirNode::new(
                        member_name,
                        NodeKind::EnumVariant,
                        path.to_path_buf(),
                        node_span(&child),
                        language,
                    );
                    let variant_id = variant_node.id;
                    output.add_node(variant_node);
                    output.add_edge(enum_id, variant_id, GirEdge::new(EdgeKind::Contains));
                }
            }
        }
    }
}

// ── Variables ───────────────────────────────────────────────

fn extract_variable_declaration(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    // Determine if this is `const`
    let decl_text = node_text(node, source);
    let is_const = decl_text.starts_with("const ");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            let name = node_text(&name_node, source);
            if name.is_empty() {
                continue;
            }

            // Check if the value is an arrow function or function expression
            if let Some(value) = child.child_by_field_name("value") {
                if value.kind() == "arrow_function" || value.kind() == "function" || value.kind() == "function_expression" {
                    extract_arrow_function(
                        &value, &name, source, path, parent_id, output, language, exported_names,
                    );
                    continue;
                }
            }

            let kind = if is_const {
                NodeKind::Constant
            } else {
                NodeKind::Variable
            };

            let span = node_span(&child);
            let visibility = if exported_names.contains(&name) {
                Visibility::Public
            } else {
                Visibility::Internal
            };

            let var_node = GirNode {
                id: SymbolId::new(path, &name, kind, span.start_line),
                name,
                kind,
                file_path: path.to_path_buf(),
                span,
                visibility,
                language,
                signature: None,
                complexity: None,
                confidence: 1.0,
                doc: None,
                coverage: None,
            };
            let var_id = var_node.id;
            output.add_node(var_node);
            output.add_edge(parent_id, var_id, GirEdge::new(EdgeKind::Contains));
        }
    }
}

// ── Imports ─────────────────────────────────────────────────

fn extract_import(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let text = node_text(node, source);
    let span = node_span(node);

    // Extract the module source
    let module_name = node
        .child_by_field_name("source")
        .map(|n| {
            let t = node_text(&n, source);
            t.trim_matches('\'').trim_matches('"').to_string()
        })
        .unwrap_or_default();

    // Collect imported items
    let mut items = Vec::new();
    let mut alias = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut clause_cursor = child.walk();
                for clause_child in child.children(&mut clause_cursor) {
                    match clause_child.kind() {
                        "identifier" => {
                            // `import foo from 'bar'` — default import
                            alias = Some(node_text(&clause_child, source));
                        }
                        "named_imports" => {
                            // `import { x, y } from 'bar'`
                            let mut named_cursor = clause_child.walk();
                            for spec in clause_child.children(&mut named_cursor) {
                                if spec.kind() == "import_specifier" {
                                    if let Some(name_node) = spec.child_by_field_name("name") {
                                        items.push(node_text(&name_node, source));
                                    }
                                }
                            }
                        }
                        "namespace_import" => {
                            // `import * as foo from 'bar'`
                            if let Some(name_node) = clause_child.child_by_field_name("name") {
                                alias = Some(format!("* as {}", node_text(&name_node, source)));
                            } else if clause_child.child_count() >= 3 {
                                // Fallback: try the last child which should be the identifier
                                if let Some(last) = clause_child.child(clause_child.child_count() - 1) {
                                    alias = Some(format!("* as {}", node_text(&last, source)));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let import_node = GirNode {
        id: SymbolId::new(path, &text, NodeKind::Import, span.start_line),
        name: module_name.clone(),
        kind: NodeKind::Import,
        file_path: path.to_path_buf(),
        span,
        visibility: Visibility::Internal,
        language,
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
            alias,
            items,
        }),
    );
}

// ── Exports ─────────────────────────────────────────────────

fn extract_export(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
    exported_names: &[String],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                extract_function(&child, source, path, parent_id, output, language, false, exported_names);
            }
            "generator_function_declaration" => {
                extract_function(&child, source, path, parent_id, output, language, false, exported_names);
            }
            "class_declaration" => {
                extract_class(&child, source, path, parent_id, output, language, exported_names);
            }
            "interface_declaration" => {
                extract_interface(&child, source, path, parent_id, output, language, exported_names);
            }
            "type_alias_declaration" => {
                extract_type_alias(&child, source, path, parent_id, output, language, exported_names);
            }
            "enum_declaration" => {
                extract_enum(&child, source, path, parent_id, output, language, exported_names);
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_variable_declaration(&child, source, path, parent_id, output, language, exported_names);
            }
            _ => {}
        }
    }
}

// ── Expression statement (module.exports, require) ──────────

fn extract_expression_statement(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let text = node_text(node, source);

    // Handle `module.exports = ...` as an export
    if text.starts_with("module.exports") {
        let span = node_span(node);
        let import_node = GirNode {
            id: SymbolId::new(path, "module.exports", NodeKind::Variable, span.start_line),
            name: "module.exports".to_string(),
            kind: NodeKind::Variable,
            file_path: path.to_path_buf(),
            span,
            visibility: Visibility::Public,
            language,
            signature: Some(text.clone()),
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        };
        let var_id = import_node.id;
        output.add_node(import_node);
        output.add_edge(parent_id, var_id, GirEdge::new(EdgeKind::Contains));
    }

    // Handle `const x = require('y')` — already handled in variable_declaration,
    // but standalone require() calls are treated as imports
    if text.contains("require(") && !text.starts_with("const ") && !text.starts_with("let ") && !text.starts_with("var ") {
        let span = node_span(node);
        let import_node = GirNode {
            id: SymbolId::new(path, &text, NodeKind::Import, span.start_line),
            name: text.clone(),
            kind: NodeKind::Import,
            file_path: path.to_path_buf(),
            span,
            visibility: Visibility::Internal,
            language,
            signature: Some(text),
            complexity: None,
            confidence: 0.8,
            doc: None,
            coverage: None,
        };
        let import_id = import_node.id;
        output.add_node(import_node);
        output.add_edge(parent_id, import_id, GirEdge::new(EdgeKind::Contains));
    }
}

// ── Parameters ──────────────────────────────────────────────

fn extract_parameters(
    params_node: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let mut cursor = params_node.walk();
    for param in params_node.children(&mut cursor) {
        let name = match param.kind() {
            "identifier" => node_text(&param, source),
            "required_parameter" | "optional_parameter" => {
                param
                    .child_by_field_name("pattern")
                    .or_else(|| param.child_by_field_name("name"))
                    .or_else(|| param.child(0))
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            "rest_pattern" => {
                // ...args
                param
                    .child(1)
                    .or_else(|| param.child(0))
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            "assignment_pattern" => {
                // param = defaultValue
                param
                    .child_by_field_name("left")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            "formal_parameters" => continue,
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
            language,
        );
        let param_id = param_node.id;
        output.add_node(param_node);
        output.add_edge(func_id, param_id, GirEdge::new(EdgeKind::Contains));

        // Extract type annotation
        if let Some(type_ann) = param.child_by_field_name("type") {
            let type_name = node_text(&type_ann, source);
            let tn = GirNode::new(
                type_name,
                NodeKind::TypeAlias,
                path.to_path_buf(),
                node_span(&type_ann),
                language,
            );
            let type_id = tn.id;
            output.add_node(tn);
            output.add_edge(param_id, type_id, GirEdge::new(EdgeKind::ParamType));
        }
    }
}

// ── Decorators ──────────────────────────────────────────────

fn extract_decorators(
    node: &Node,
    source: &[u8],
    path: &Path,
    target_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    // Decorators appear as preceding siblings or children of the node
    // In tree-sitter-typescript, decorators are child nodes of type "decorator"
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let dec_text = node_text(&child, source);
            let dec_name = dec_text.trim_start_matches('@').trim().to_string();
            let dec_node = GirNode::new(
                dec_name,
                NodeKind::Decorator,
                path.to_path_buf(),
                node_span(&child),
                language,
            );
            let dec_id = dec_node.id;
            output.add_node(dec_node);
            output.add_edge(target_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
        }
    }
}

// ── Return types ────────────────────────────────────────────

fn extract_return_type(
    ret_node: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let type_name = node_text(ret_node, source);
    // Strip leading `: ` if present (type_annotation nodes may include the colon)
    let clean_name = type_name.trim_start_matches(':').trim().to_string();
    if clean_name.is_empty() {
        return;
    }
    let type_node = GirNode::new(
        clean_name,
        NodeKind::TypeAlias,
        path.to_path_buf(),
        node_span(ret_node),
        language,
    );
    let type_id = type_node.id;
    output.add_node(type_node);
    output.add_edge(func_id, type_id, GirEdge::new(EdgeKind::ReturnsType));
}

// ── Call extraction ─────────────────────────────────────────

fn extract_calls_from_body(
    body: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
    language: Language,
) {
    let mut stack = vec![*body];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let call_name = node_text(&func_node, source);

                if !is_noise_builtin(&call_name) && !is_noise_method_call(&call_name) {
                    let call_target = GirNode::new(
                        call_name.clone(),
                        NodeKind::Function,
                        path.to_path_buf(),
                        node_span(&func_node),
                        language,
                    );
                    let target_id = call_target.id;
                    output.add_node(call_target);

                    let is_dynamic = call_name.contains('.');
                    let confidence = if is_dynamic { 0.7 } else { 0.9 };
                    output.add_edge(
                        func_id,
                        target_id,
                        GirEdge::new(EdgeKind::Calls)
                            .with_confidence(confidence)
                            .with_metadata(EdgeMetadata::Call { is_dynamic }),
                    );
                }
            }
        }

        // Don't recurse into nested function/class definitions
        let dominated = matches!(
            node.kind(),
            "function_declaration" | "function" | "function_expression"
                | "arrow_function" | "class_declaration" | "class"
        );
        if !dominated || node == *body {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn method_visibility(node: &Node, source: &[u8]) -> Visibility {
    // Check for access modifiers: private, protected, public
    let text = node_text(node, source);
    if text.starts_with("private ") || text.starts_with("private\t") {
        Visibility::Private
    } else if text.starts_with("protected ") || text.starts_with("protected\t") {
        Visibility::Internal
    } else if text.starts_with("public ") || text.starts_with("public\t") {
        Visibility::Public
    } else {
        // Also check child nodes for accessibility modifiers
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "accessibility_modifier" {
                let modifier = node_text(&child, source);
                return match modifier.as_str() {
                    "private" => Visibility::Private,
                    "protected" => Visibility::Internal,
                    "public" => Visibility::Public,
                    _ => Visibility::Public,
                };
            }
        }
        Visibility::Public
    }
}

fn build_function_signature(node: &Node, source: &[u8], name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, source))
        .unwrap_or_else(|| "()".to_string());

    let ret = node
        .child_by_field_name("return_type")
        .map(|r| {
            let t = node_text(&r, source);
            if t.starts_with(':') {
                t
            } else {
                format!(": {t}")
            }
        })
        .unwrap_or_default();

    format!("function {name}{params}{ret}")
}

fn build_arrow_signature(node: &Node, source: &[u8], name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, source))
        .or_else(|| {
            node.child_by_field_name("parameter")
                .map(|p| format!("({})", node_text(&p, source)))
        })
        .unwrap_or_else(|| "()".to_string());

    let ret = node
        .child_by_field_name("return_type")
        .map(|r| {
            let t = node_text(&r, source);
            if t.starts_with(':') {
                t
            } else {
                format!(": {t}")
            }
        })
        .unwrap_or_default();

    format!("const {name} = {params}{ret} => ...")
}

fn extract_jsdoc(node: &Node, source: &[u8]) -> Option<String> {
    // JSDoc is typically a comment node preceding the declaration.
    // In tree-sitter, comments are siblings. Check the previous sibling.
    let prev = node.prev_sibling()?;
    if prev.kind() == "comment" {
        let text = node_text(&prev, source);
        if text.starts_with("/**") {
            // Strip /** and */ and leading * on each line
            let cleaned = text
                .trim_start_matches("/**")
                .trim_end_matches("*/")
                .lines()
                .map(|line| line.trim().trim_start_matches('*').trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn is_noise_builtin(name: &str) -> bool {
    matches!(
        name,
        "console.log"
            | "console.error"
            | "console.warn"
            | "console.info"
            | "console.debug"
            | "JSON.stringify"
            | "JSON.parse"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "String"
            | "Number"
            | "Boolean"
            | "Array"
            | "Object"
            | "Math.floor"
            | "Math.ceil"
            | "Math.round"
            | "Math.max"
            | "Math.min"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::NodeKind;

    #[test]
    fn parse_simple_function() {
        let source = r#"
function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "greet");
    }

    #[test]
    fn parse_arrow_function() {
        let source = r#"
const add = (a: number, b: number): number => a + b;
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "add");
    }

    #[test]
    fn parse_class_with_methods() {
        let source = r#"
class Dog extends Animal {
    name: string;

    constructor(name: string) {
        super(name);
        this.name = name;
    }

    bark(): string {
        return "Woof!";
    }
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
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
    fn parse_interface() {
        let source = r#"
interface Greetable {
    name: string;
    greet(): string;
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let ifaces: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Interface)
            .collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Greetable");
    }

    #[test]
    fn parse_enum() {
        let source = r#"
enum Direction {
    Up,
    Down,
    Left,
    Right,
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let enums: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Direction");
    }

    #[test]
    fn parse_imports() {
        let source = r#"
import { readFile } from 'fs';
import path from 'path';
import * as http from 'http';
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let imports: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn parse_exported_function_visibility() {
        let source = r#"
export function publicFn(): void {}
function privateFn(): void {}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 2);

        let public_fn = funcs.iter().find(|f| f.name == "publicFn").unwrap();
        assert_eq!(public_fn.visibility, Visibility::Public);

        let private_fn = funcs.iter().find(|f| f.name == "privateFn").unwrap();
        assert_eq!(private_fn.visibility, Visibility::Internal);
    }

    #[test]
    fn parse_javascript_file() {
        let source = r#"
function hello() {
    return "world";
}
const x = 42;
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.js"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);

        let consts: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constant)
            .collect();
        assert_eq!(consts.len(), 1);
    }

    #[test]
    fn parse_type_alias() {
        let source = r#"
type Point = { x: number; y: number };
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        let types: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::TypeAlias && n.name == "Point")
            .collect();
        assert_eq!(types.len(), 1);
    }

    // ── Edge case tests ───────────────────────────────────

    #[test]
    fn parse_empty_file() {
        let output = TypeScriptFrontend::new()
            .parse(Path::new("empty.ts"), "")
            .unwrap();
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn parse_comments_only() {
        let source = "// This is a comment\n/* Block comment */\n";
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function || n.kind == NodeKind::Class)
            .collect();
        assert!(funcs.is_empty());
    }

    #[test]
    fn parse_async_function() {
        let source = r#"
async function fetchData(url: string): Promise<Response> {
    return await fetch(url);
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        // fetchData is the real function; fetch() creates a phantom call target
        assert!(funcs.iter().any(|f| f.name == "fetchData"));
    }

    #[test]
    fn parse_class_with_generics() {
        let source = r#"
class Container<T> {
    value: T;
    constructor(val: T) {
        this.value = val;
    }
    get(): T {
        return this.value;
    }
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();
        let classes: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Container");
    }

    #[test]
    fn parse_jsx_file() {
        let source = r#"
function App() {
    return <div>Hello</div>;
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("app.jsx"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "App");
    }

    #[test]
    fn parse_tsx_file() {
        let source = r#"
interface Props {
    name: string;
}

function Greeting({ name }: Props) {
    return <h1>Hello, {name}</h1>;
}

export default Greeting;
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("greeting.tsx"), source)
            .unwrap();
        let ifaces: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Interface)
            .collect();
        assert_eq!(ifaces.len(), 1);
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
    }

    #[test]
    fn parse_mjs_extension() {
        let source = "export function hello() { return 42; }\n";
        let output = TypeScriptFrontend::new()
            .parse(Path::new("module.mjs"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
    }

    #[test]
    fn parse_call_expressions() {
        let source = r#"
function main() {
    console.log("hello");
    helper();
}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();
        let calls: Vec<_> = output.edges.iter()
            .filter(|e| e.2.kind == EdgeKind::Calls)
            .collect();
        assert!(calls.len() >= 1);
    }

    #[test]
    fn parse_multiple_exports() {
        let source = r#"
export const PI = 3.14;
export function add(a: number, b: number) { return a + b; }
export class Calculator {}
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();
        let exported: Vec<_> = output.nodes.iter()
            .filter(|n| n.visibility == Visibility::Public && n.kind != NodeKind::File)
            .collect();
        assert!(exported.len() >= 2);
    }

    #[test]
    fn parse_generic_function_with_constraints() {
        let source = r#"
function identity<T extends Serializable>(arg: T): T {
    return arg;
}

interface Pair<K, V> {
    key: K;
    value: V;
}

type Result<T, E = Error> = { ok: T } | { err: E };
"#;
        let output = TypeScriptFrontend::new()
            .parse(Path::new("test.ts"), source)
            .unwrap();

        // Generic function should be parsed with its name
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "identity");

        // Generic interface should be parsed
        let ifaces: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Interface)
            .collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Pair");

        // Generic type alias should be parsed
        let types: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::TypeAlias && n.name == "Result")
            .collect();
        assert_eq!(types.len(), 1);
    }
}
