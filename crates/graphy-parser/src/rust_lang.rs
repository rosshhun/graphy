use std::path::Path;

use anyhow::{Context, Result};
use graphy_core::{
    EdgeKind, EdgeMetadata, GirEdge, GirNode, Language, NodeKind, ParseOutput, SymbolId,
    Visibility,
};
use tree_sitter::{Node, Parser};

use crate::frontend::LanguageFrontend;
use crate::helpers::{is_noise_method_call, node_span, node_text};

/// Frontend for Rust (.rs) files.
pub struct RustFrontend;

impl RustFrontend {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageFrontend for RustFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .context("Failed to set Rust language")?;

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
            language: Language::Rust,
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
        "function_item" => {
            extract_function(node, source, path, parent_id, output, false);
        }
        "struct_item" => {
            extract_struct(node, source, path, parent_id, output);
        }
        "enum_item" => {
            extract_enum(node, source, path, parent_id, output);
        }
        "trait_item" => {
            extract_trait(node, source, path, parent_id, output);
        }
        "impl_item" => {
            extract_impl(node, source, path, parent_id, output);
        }
        "type_item" => {
            extract_type_alias(node, source, path, parent_id, output);
        }
        "const_item" => {
            extract_const(node, source, path, parent_id, output);
        }
        "static_item" => {
            extract_static(node, source, path, parent_id, output);
        }
        "mod_item" => {
            extract_mod(node, source, path, parent_id, output);
        }
        "use_declaration" => {
            extract_use(node, source, path, parent_id, output);
        }
        "attribute_item" => {
            // Top-level attributes (like #![...]) — skip for now
        }
        "macro_definition" => {
            extract_macro_def(node, source, path, parent_id, output);
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
    is_method: bool,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);

    let kind = if is_method {
        if name == "new" {
            NodeKind::Constructor
        } else {
            NodeKind::Method
        }
    } else {
        NodeKind::Function
    };

    let visibility = extract_visibility(node, source);
    let sig = build_function_signature(node, source, &name);
    let doc = extract_doc_comment(node, source);
    let generics = extract_generics(node, source);

    let signature = if let Some(g) = &generics {
        Some(format!("{sig}{g}"))
    } else {
        Some(sig)
    };

    let func_node = GirNode {
        id: SymbolId::new(path, &name, kind, span.start_line),
        name: name.clone(),
        kind,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature,
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
        extract_parameters(&params, source, path, func_id, output);
    }

    // Extract return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        let type_name = node_text(&ret, source);
        // Strip leading `-> ` if present
        let clean_name = type_name.trim_start_matches("->").trim().to_string();
        if !clean_name.is_empty() {
            let type_node = GirNode::new(
                clean_name,
                NodeKind::TypeAlias,
                path.to_path_buf(),
                node_span(&ret),
                Language::Rust,
            );
            let type_id = type_node.id;
            output.add_node(type_node);
            output.add_edge(func_id, type_id, GirEdge::new(EdgeKind::ReturnsType));
        }
    }

    // Walk function body for calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, path, func_id, output);
    }

    // Extract attributes as decorators
    extract_attributes_as_decorators(node, source, path, func_id, output);
}

// ── Structs ─────────────────────────────────────────────────

fn extract_struct(
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
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comment(node, source);

    let struct_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Struct, span.start_line),
        name: name.clone(),
        kind: NodeKind::Struct,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(format!("struct {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let struct_id = struct_node.id;
    output.add_node(struct_node);
    output.add_edge(parent_id, struct_id, GirEdge::new(EdgeKind::Contains));

    // Extract struct fields — prefer the `body` field name, fallback to
    // searching for field_declaration_list among children
    let body_node = node.child_by_field_name("body");
    if let Some(body) = body_node {
        extract_struct_fields(&body, source, path, struct_id, output);
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                extract_struct_fields(&child, source, path, struct_id, output);
                break;
            }
        }
    }

    // Extract derive macros and other attributes as decorators
    extract_attributes_as_decorators(node, source, path, struct_id, output);
}

fn extract_struct_fields(
    body: &Node,
    source: &[u8],
    path: &Path,
    struct_id: SymbolId,
    output: &mut ParseOutput,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let field_name = node_text(&name_node, source);
                let field_span = node_span(&child);
                let field_vis = extract_visibility(&child, source);

                let field_node = GirNode {
                    id: SymbolId::new(path, &field_name, NodeKind::Field, field_span.start_line),
                    name: field_name,
                    kind: NodeKind::Field,
                    file_path: path.to_path_buf(),
                    span: field_span,
                    visibility: field_vis,
                    language: Language::Rust,
                    signature: None,
                    complexity: None,
                    confidence: 1.0,
                    doc: None,
                    coverage: None,
                };
                let field_id = field_node.id;
                output.add_node(field_node);
                output.add_edge(struct_id, field_id, GirEdge::new(EdgeKind::Contains));

                // Extract field type
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(&type_node, source);
                    let tn = GirNode::new(
                        type_name,
                        NodeKind::TypeAlias,
                        path.to_path_buf(),
                        node_span(&type_node),
                        Language::Rust,
                    );
                    let type_id = tn.id;
                    output.add_node(tn);
                    output.add_edge(field_id, type_id, GirEdge::new(EdgeKind::FieldType));
                }
            }
        }
    }
}

// ── Enums ───────────────────────────────────────────────────

fn extract_enum(
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
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comment(node, source);

    let enum_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Enum, span.start_line),
        name: name.clone(),
        kind: NodeKind::Enum,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(format!("enum {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let enum_id = enum_node.id;
    output.add_node(enum_node);
    output.add_edge(parent_id, enum_id, GirEdge::new(EdgeKind::Contains));

    // Extract enum variants
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_variant" {
                if let Some(vname) = child.child_by_field_name("name") {
                    let variant_name = node_text(&vname, source);
                    let variant_node = GirNode::new(
                        variant_name,
                        NodeKind::EnumVariant,
                        path.to_path_buf(),
                        node_span(&child),
                        Language::Rust,
                    );
                    let variant_id = variant_node.id;
                    output.add_node(variant_node);
                    output.add_edge(enum_id, variant_id, GirEdge::new(EdgeKind::Contains));
                }
            }
        }
    }

    // Extract derive macros
    extract_attributes_as_decorators(node, source, path, enum_id, output);
}

// ── Traits ──────────────────────────────────────────────────

fn extract_trait(
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
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comment(node, source);

    let trait_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Trait, span.start_line),
        name: name.clone(),
        kind: NodeKind::Trait,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(format!("trait {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let trait_id = trait_node.id;
    output.add_node(trait_node);
    output.add_edge(parent_id, trait_id, GirEdge::new(EdgeKind::Contains));

    // Extract trait methods (function signatures in body)
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_item" => {
                    extract_function(&child, source, path, trait_id, output, true);
                }
                "function_signature_item" => {
                    extract_trait_method_signature(&child, source, path, trait_id, output);
                }
                "type_item" => {
                    extract_type_alias(&child, source, path, trait_id, output);
                }
                "const_item" => {
                    extract_const(&child, source, path, trait_id, output);
                }
                _ => {}
            }
        }
    }

    // Extract supertraits (trait bounds after colon)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "trait_bounds" {
            let mut bounds_cursor = child.walk();
            for bound in child.children(&mut bounds_cursor) {
                if bound.kind() == "type_identifier" || bound.kind() == "scoped_type_identifier" || bound.kind() == "generic_type" {
                    let bound_name = node_text(&bound, source);
                    let bound_node = GirNode::new(
                        bound_name,
                        NodeKind::Trait,
                        path.to_path_buf(),
                        node_span(&bound),
                        Language::Rust,
                    );
                    let bound_id = bound_node.id;
                    output.add_node(bound_node);
                    output.add_edge(
                        trait_id,
                        bound_id,
                        GirEdge::new(EdgeKind::Inherits)
                            .with_metadata(EdgeMetadata::Inheritance { depth: 1 }),
                    );
                }
            }
        }
    }
}

fn extract_trait_method_signature(
    node: &Node,
    source: &[u8],
    path: &Path,
    trait_id: SymbolId,
    output: &mut ParseOutput,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let sig = build_function_signature(node, source, &name);

    let method_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Method, span.start_line),
        name,
        kind: NodeKind::Method,
        file_path: path.to_path_buf(),
        span,
        visibility: Visibility::Public,
        language: Language::Rust,
        signature: Some(sig),
        complexity: None,
        confidence: 1.0,
        doc: extract_doc_comment(node, source),
        coverage: None,
    };
    let method_id = method_node.id;
    output.add_node(method_node);
    output.add_edge(trait_id, method_id, GirEdge::new(EdgeKind::Contains));

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_parameters(&params, source, path, method_id, output);
    }

    // Extract return type
    if let Some(ret) = node.child_by_field_name("return_type") {
        let type_name = node_text(&ret, source);
        let clean_name = type_name.trim_start_matches("->").trim().to_string();
        if !clean_name.is_empty() {
            let type_node = GirNode::new(
                clean_name,
                NodeKind::TypeAlias,
                path.to_path_buf(),
                node_span(&ret),
                Language::Rust,
            );
            let type_id = type_node.id;
            output.add_node(type_node);
            output.add_edge(method_id, type_id, GirEdge::new(EdgeKind::ReturnsType));
        }
    }
}

// ── Impl Blocks ─────────────────────────────────────────────

fn extract_impl(
    node: &Node,
    source: &[u8],
    path: &Path,
    _parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    // Determine the type being implemented and optional trait
    let impl_type = node
        .child_by_field_name("type")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();

    let impl_trait = node
        .child_by_field_name("trait")
        .map(|n| node_text(&n, source));

    if impl_type.is_empty() {
        return;
    }

    let span = node_span(node);

    // Try to find the existing struct/enum/trait definition in the output
    // so impl block methods attach to the real definition node, not duplicates.
    let type_id = if let Some(existing) = output.nodes.iter().find(|n| {
        n.name == impl_type
            && n.file_path == path
            && matches!(
                n.kind,
                NodeKind::Struct | NodeKind::Enum | NodeKind::Trait | NodeKind::Class
            )
    }) {
        existing.id
    } else {
        // No definition found (e.g., impl for external type) — create a synthetic node
        let type_node = GirNode::new(
            impl_type.clone(),
            NodeKind::Struct,
            path.to_path_buf(),
            span,
            Language::Rust,
        );
        let id = type_node.id;
        output.add_node(type_node);
        id
    };

    // If this is a trait impl, create the Implements edge
    if let Some(ref trait_name) = impl_trait {
        let trait_node = GirNode::new(
            trait_name.clone(),
            NodeKind::Trait,
            path.to_path_buf(),
            span,
            Language::Rust,
        );
        let trait_id = trait_node.id;
        output.add_node(trait_node);
        output.add_edge(type_id, trait_id, GirEdge::new(EdgeKind::Implements));
    }

    // Walk impl body for methods
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_item" => {
                    extract_function(&child, source, path, type_id, output, true);
                }
                "type_item" => {
                    extract_type_alias(&child, source, path, type_id, output);
                }
                "const_item" => {
                    extract_const(&child, source, path, type_id, output);
                }
                _ => {}
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
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(&name_node, source);
    let span = node_span(node);
    let visibility = extract_visibility(node, source);
    let full_text = node_text(node, source);

    let type_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::TypeAlias, span.start_line),
        name,
        kind: NodeKind::TypeAlias,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(full_text),
        complexity: None,
        confidence: 1.0,
        doc: extract_doc_comment(node, source),
        coverage: None,
    };
    let type_id = type_node.id;
    output.add_node(type_node);
    output.add_edge(parent_id, type_id, GirEdge::new(EdgeKind::Contains));
}

// ── Constants ───────────────────────────────────────────────

fn extract_const(
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
    let visibility = extract_visibility(node, source);

    let const_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Constant, span.start_line),
        name,
        kind: NodeKind::Constant,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(node_text(node, source)),
        complexity: None,
        confidence: 1.0,
        doc: extract_doc_comment(node, source),
        coverage: None,
    };
    let const_id = const_node.id;
    output.add_node(const_node);
    output.add_edge(parent_id, const_id, GirEdge::new(EdgeKind::Contains));

    // Extract type annotation
    if let Some(type_ann) = node.child_by_field_name("type") {
        let type_name = node_text(&type_ann, source);
        let tn = GirNode::new(
            type_name,
            NodeKind::TypeAlias,
            path.to_path_buf(),
            node_span(&type_ann),
            Language::Rust,
        );
        let type_id = tn.id;
        output.add_node(tn);
        output.add_edge(const_id, type_id, GirEdge::new(EdgeKind::FieldType));
    }
}

// ── Statics ─────────────────────────────────────────────────

fn extract_static(
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
    let visibility = extract_visibility(node, source);

    let static_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Variable, span.start_line),
        name,
        kind: NodeKind::Variable,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(node_text(node, source)),
        complexity: None,
        confidence: 1.0,
        doc: extract_doc_comment(node, source),
        coverage: None,
    };
    let static_id = static_node.id;
    output.add_node(static_node);
    output.add_edge(parent_id, static_id, GirEdge::new(EdgeKind::Contains));
}

// ── Modules ─────────────────────────────────────────────────

fn extract_mod(
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
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comment(node, source);

    let mod_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Module, span.start_line),
        name: name.clone(),
        kind: NodeKind::Module,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
        signature: Some(format!("mod {name}")),
        complexity: None,
        confidence: 1.0,
        doc,
        coverage: None,
    };
    let mod_id = mod_node.id;
    output.add_node(mod_node);
    output.add_edge(parent_id, mod_id, GirEdge::new(EdgeKind::Contains));

    // Extract attributes (e.g. #[cfg(test)]) as decorators on the module
    extract_attributes_as_decorators(node, source, path, mod_id, output);

    // If this is an inline module (has a body), walk its children
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            extract_node(&child, source, path, mod_id, output);
        }
    }
}

// ── Use Declarations ────────────────────────────────────────

fn extract_use(
    node: &Node,
    source: &[u8],
    path: &Path,
    parent_id: SymbolId,
    output: &mut ParseOutput,
) {
    let text = node_text(node, source);
    let span = node_span(node);
    let visibility = extract_visibility(node, source);

    // Extract the path being imported
    let mut items = Vec::new();
    collect_use_items(node, source, &mut items);

    let import_node = GirNode {
        id: SymbolId::new(path, &text, NodeKind::Import, span.start_line),
        name: text.clone(),
        kind: NodeKind::Import,
        file_path: path.to_path_buf(),
        span,
        visibility,
        language: Language::Rust,
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

fn collect_use_items(node: &Node, source: &[u8], items: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_as_clause" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    items.push(node_text(&path_node, source));
                }
            }
            "use_list" => {
                let mut list_cursor = child.walk();
                for list_child in child.children(&mut list_cursor) {
                    if list_child.kind() == "identifier" || list_child.kind() == "scoped_identifier" {
                        items.push(node_text(&list_child, source));
                    } else if list_child.kind() == "use_as_clause" {
                        if let Some(path_node) = list_child.child_by_field_name("path") {
                            items.push(node_text(&path_node, source));
                        }
                    }
                }
            }
            "scoped_use_list" => {
                collect_use_items(&child, source, items);
            }
            "identifier" => {
                items.push(node_text(&child, source));
            }
            "scoped_identifier" => {
                items.push(node_text(&child, source));
            }
            _ => {
                // Recurse for nested structures
                if child.child_count() > 0 {
                    collect_use_items(&child, source, items);
                }
            }
        }
    }
}

// ── Macro definitions ───────────────────────────────────────

fn extract_macro_def(
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

    let macro_node = GirNode {
        id: SymbolId::new(path, &name, NodeKind::Function, span.start_line),
        name: format!("{name}!"),
        kind: NodeKind::Function,
        file_path: path.to_path_buf(),
        span,
        visibility: extract_visibility(node, source),
        language: Language::Rust,
        signature: Some(format!("macro_rules! {name}")),
        complexity: None,
        confidence: 1.0,
        doc: extract_doc_comment(node, source),
        coverage: None,
    };
    let macro_id = macro_node.id;
    output.add_node(macro_node);
    output.add_edge(parent_id, macro_id, GirEdge::new(EdgeKind::Contains));
}

// ── Parameters ──────────────────────────────────────────────

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
            "parameter" => {
                param
                    .child_by_field_name("pattern")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default()
            }
            "self_parameter" => {
                node_text(&param, source)
            }
            "identifier" => node_text(&param, source),
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
            Language::Rust,
        );
        let param_id = param_node.id;
        output.add_node(param_node);
        output.add_edge(func_id, param_id, GirEdge::new(EdgeKind::Contains));

        // Extract parameter type
        if let Some(type_ann) = param.child_by_field_name("type") {
            let type_name = node_text(&type_ann, source);
            let tn = GirNode::new(
                type_name,
                NodeKind::TypeAlias,
                path.to_path_buf(),
                node_span(&type_ann),
                Language::Rust,
            );
            let type_id = tn.id;
            output.add_node(tn);
            output.add_edge(param_id, type_id, GirEdge::new(EdgeKind::ParamType));
        }
    }
}

// ── Attributes / Decorators ─────────────────────────────────

fn extract_attributes_as_decorators(
    node: &Node,
    source: &[u8],
    path: &Path,
    target_id: SymbolId,
    output: &mut ParseOutput,
) {
    // Look for attribute_item siblings preceding this node
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let attr_text = node_text(&sibling, source);
            // Strip outer #[...] or #![...]
            let inner = attr_text
                .trim_start_matches("#![")
                .trim_start_matches("#[")
                .trim_end_matches(']')
                .trim()
                .to_string();

            // If it's a derive, expand into individual derive decorators
            if inner.starts_with("derive(") {
                let derives = inner
                    .trim_start_matches("derive(")
                    .trim_end_matches(')')
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                for derive_name in derives {
                    let dec_node = GirNode::new(
                        format!("derive({derive_name})"),
                        NodeKind::Decorator,
                        path.to_path_buf(),
                        node_span(&sibling),
                        Language::Rust,
                    );
                    let dec_id = dec_node.id;
                    output.add_node(dec_node);
                    output.add_edge(target_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
                }
            } else {
                let dec_node = GirNode::new(
                    inner,
                    NodeKind::Decorator,
                    path.to_path_buf(),
                    node_span(&sibling),
                    Language::Rust,
                );
                let dec_id = dec_node.id;
                output.add_node(dec_node);
                output.add_edge(target_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));
            }
            prev = sibling.prev_sibling();
        } else if sibling.kind() == "line_comment" || sibling.kind() == "block_comment" {
            // Skip doc comments (they precede attributes sometimes)
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }
}

// ── Call extraction ─────────────────────────────────────────

fn extract_calls_from_body(
    body: &Node,
    source: &[u8],
    path: &Path,
    func_id: SymbolId,
    output: &mut ParseOutput,
) {
    let mut stack = vec![*body];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child_by_field_name("function") {
                let call_name = node_text(&func_node, source);

                if !is_noise_call(&call_name) && !is_noise_method_call(&call_name) {
                    let call_target = GirNode::new(
                        call_name.clone(),
                        NodeKind::Function,
                        path.to_path_buf(),
                        node_span(&func_node),
                        Language::Rust,
                    );
                    let target_id = call_target.id;
                    output.add_node(call_target);

                    let is_dynamic = call_name.contains("::");
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

        // Also handle macro invocations
        if node.kind() == "macro_invocation" {
            if let Some(macro_node) = node.child_by_field_name("macro") {
                let macro_name = node_text(&macro_node, source);
                if !is_noise_macro(&macro_name) {
                    let call_target = GirNode::new(
                        format!("{macro_name}!"),
                        NodeKind::Function,
                        path.to_path_buf(),
                        node_span(&macro_node),
                        Language::Rust,
                    );
                    let target_id = call_target.id;
                    output.add_node(call_target);
                    output.add_edge(
                        func_id,
                        target_id,
                        GirEdge::new(EdgeKind::Calls)
                            .with_confidence(0.8)
                            .with_metadata(EdgeMetadata::Call { is_dynamic: false }),
                    );
                }
            }
        }

        // Don't recurse into nested function definitions.
        // Closures ARE traversed because they don't get their own GirNode —
        // their calls should be attributed to the enclosing function.
        let is_nested = node.kind() == "function_item";
        if !is_nested || node == *body {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn extract_visibility(node: &Node, source: &[u8]) -> Visibility {
    // Look for a visibility_modifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let vis_text = node_text(&child, source);
            return match vis_text.as_str() {
                "pub" => Visibility::Public,
                "pub(crate)" => Visibility::Internal,
                "pub(super)" => Visibility::Internal,
                _ if vis_text.starts_with("pub(") => Visibility::Internal,
                _ => Visibility::Private,
            };
        }
    }
    Visibility::Private
}

fn extract_generics(node: &Node, source: &[u8]) -> Option<String> {
    node.child_by_field_name("type_parameters")
        .map(|n| node_text(&n, source))
}

fn build_function_signature(node: &Node, source: &[u8], name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, source))
        .unwrap_or_else(|| "()".to_string());

    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(" {}", node_text(&r, source)))
        .unwrap_or_default();

    let generics = node
        .child_by_field_name("type_parameters")
        .map(|g| node_text(&g, source))
        .unwrap_or_default();

    format!("fn {name}{generics}{params}{ret}")
}

fn extract_doc_comment(node: &Node, source: &[u8]) -> Option<String> {
    // Rust doc comments are `///` or `//!` comment lines preceding the item
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        if sibling.kind() == "line_comment" {
            let text = node_text(&sibling, source);
            if text.starts_with("///") {
                let content = text.trim_start_matches("///").trim();
                doc_lines.push(content.to_string());
                prev = sibling.prev_sibling();
                continue;
            } else if text.starts_with("//!") {
                let content = text.trim_start_matches("//!").trim();
                doc_lines.push(content.to_string());
                prev = sibling.prev_sibling();
                continue;
            }
        } else if sibling.kind() == "attribute_item" {
            // Skip attributes between doc comments and the item
            prev = sibling.prev_sibling();
            continue;
        }
        break;
    }

    if doc_lines.is_empty() {
        // Also check for block doc comments: /** ... */
        if let Some(sibling) = node.prev_sibling() {
            if sibling.kind() == "block_comment" {
                let text = node_text(&sibling, source);
                if text.starts_with("/**") || text.starts_with("/*!") {
                    let cleaned = text
                        .trim_start_matches("/**")
                        .trim_start_matches("/*!")
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
        }
        return None;
    }

    // Reverse because we collected from bottom to top
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Bare function/macro names that are language builtins (no receiver).
/// Method chains on variables are handled by `is_noise_method_call()`.
fn is_noise_call(name: &str) -> bool {
    matches!(
        name,
        "println" | "print" | "eprintln" | "eprint"
            | "format" | "write" | "writeln"
            | "dbg" | "todo" | "unimplemented" | "unreachable"
            | "assert" | "assert_eq" | "assert_ne"
            | "debug_assert" | "debug_assert_eq" | "debug_assert_ne"
            | "panic" | "Ok" | "Err" | "Some" | "None"
    )
}

fn is_noise_macro(name: &str) -> bool {
    matches!(
        name,
        "println" | "print" | "eprintln" | "eprint"
            | "format" | "write" | "writeln"
            | "dbg" | "todo" | "unimplemented" | "unreachable"
            | "assert" | "assert_eq" | "assert_ne"
            | "debug_assert" | "debug_assert_eq" | "debug_assert_ne"
            | "panic" | "vec" | "cfg" | "include"
            | "env" | "concat" | "stringify"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::NodeKind;

    #[test]
    fn parse_simple_function() {
        let source = r#"
/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "add");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(funcs[0].doc.as_deref().unwrap().contains("Adds two numbers"));
    }

    #[test]
    fn parse_struct_with_fields() {
        let source = r#"
#[derive(Debug, Clone)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let structs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Point");

        let fields: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Field)
            .collect();
        assert_eq!(fields.len(), 2);

        // Check derive decorators
        let decorators: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Decorator)
            .collect();
        assert!(decorators.len() >= 2);
        assert!(decorators.iter().any(|d| d.name.contains("Debug")));
        assert!(decorators.iter().any(|d| d.name.contains("Clone")));
    }

    #[test]
    fn parse_enum_with_variants() {
        let source = r#"
pub enum Color {
    Red,
    Green,
    Blue,
    Custom(u8, u8, u8),
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let enums: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Color");

        let variants: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::EnumVariant)
            .collect();
        assert_eq!(variants.len(), 4);
    }

    #[test]
    fn parse_trait_definition() {
        let source = r#"
pub trait Drawable {
    fn draw(&self);
    fn area(&self) -> f64;
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let traits: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Trait)
            .collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Drawable");

        let methods: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert!(methods.len() >= 2);
    }

    #[test]
    fn parse_impl_block() {
        let source = r#"
struct Circle {
    radius: f64,
}

impl Circle {
    pub fn new(radius: f64) -> Self {
        Circle { radius }
    }

    pub fn area(&self) -> f64 {
        std::f64::consts::PI * self.radius * self.radius
    }
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let constructors: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constructor)
            .collect();
        assert_eq!(constructors.len(), 1);
        assert_eq!(constructors[0].name, "new");

        let methods: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "area");
    }

    #[test]
    fn parse_trait_impl() {
        let source = r#"
struct Square {
    side: f64,
}

impl Drawable for Square {
    fn draw(&self) {
        todo!()
    }

    fn area(&self) -> f64 {
        self.side * self.side
    }
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        // Check that there's an Implements edge
        let impl_edges: Vec<_> = output
            .edges
            .iter()
            .filter(|(_, _, e)| e.kind == EdgeKind::Implements)
            .collect();
        assert!(!impl_edges.is_empty());
    }

    #[test]
    fn parse_use_declarations() {
        let source = r#"
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::utils;
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let imports: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn parse_mod_declaration() {
        let source = r#"
pub mod utils;
mod internal;
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let modules: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();
        assert_eq!(modules.len(), 2);

        let pub_mod = modules.iter().find(|m| m.name == "utils").unwrap();
        assert_eq!(pub_mod.visibility, Visibility::Public);

        let priv_mod = modules.iter().find(|m| m.name == "internal").unwrap();
        assert_eq!(priv_mod.visibility, Visibility::Private);
    }

    #[test]
    fn parse_visibility() {
        let source = r#"
pub fn public_fn() {}
pub(crate) fn crate_fn() {}
fn private_fn() {}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();

        let pub_fn = funcs.iter().find(|f| f.name == "public_fn").unwrap();
        assert_eq!(pub_fn.visibility, Visibility::Public);

        let crate_fn = funcs.iter().find(|f| f.name == "crate_fn").unwrap();
        assert_eq!(crate_fn.visibility, Visibility::Internal);

        let priv_fn = funcs.iter().find(|f| f.name == "private_fn").unwrap();
        assert_eq!(priv_fn.visibility, Visibility::Private);
    }

    #[test]
    fn parse_const_and_static() {
        let source = r#"
pub const MAX_SIZE: usize = 1024;
static COUNTER: u32 = 0;
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let consts: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Constant)
            .collect();
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].name, "MAX_SIZE");

        let vars: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Variable)
            .collect();
        assert_eq!(vars.len(), 1);
    }

    #[test]
    fn parse_type_alias() {
        let source = r#"
pub type Result<T> = std::result::Result<T, MyError>;
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        let types: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::TypeAlias && n.name == "Result")
            .collect();
        assert_eq!(types.len(), 1);
    }

    // ── Edge case tests ───────────────────────────────────

    #[test]
    fn parse_empty_file() {
        let output = RustFrontend::new()
            .parse(Path::new("empty.rs"), "")
            .unwrap();
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn parse_async_function() {
        let source = r#"
pub async fn fetch(url: &str) -> Result<String> {
    todo!()
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "fetch");
    }

    #[test]
    fn parse_lifetime_annotations() {
        let source = r#"
pub fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "longest");
    }

    #[test]
    fn parse_inline_mod_with_items() {
        let source = r#"
mod inner {
    pub fn inner_fn() {}
    pub struct InnerStruct;
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        let modules: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();
        assert!(modules.iter().any(|m| m.name == "inner"));
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(funcs.iter().any(|f| f.name == "inner_fn"));
    }

    #[test]
    fn parse_cfg_test_module() {
        let source = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        // Should parse the test module and the test function
        let modules: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();
        assert!(modules.iter().any(|m| m.name == "tests"));
    }

    #[test]
    fn parse_function_with_where_clause() {
        let source = r#"
pub fn process<T>(item: T) -> String
where
    T: std::fmt::Display + Clone,
{
    format!("{}", item)
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "process");
    }

    #[test]
    fn parse_call_expressions() {
        let source = r#"
fn main() {
    let x = foo();
    bar::baz();
    obj.method();
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();
        let calls: Vec<_> = output.edges.iter()
            .filter(|e| e.2.kind == EdgeKind::Calls)
            .collect();
        assert!(calls.len() >= 1);
    }

    #[test]
    fn parse_async_fn_with_lifetime_and_generics() {
        let source = r#"
pub async fn process<'a, T: Send + Sync>(data: &'a [T]) -> Result<&'a T, Box<dyn std::error::Error>> {
    compute(data)
}
"#;
        let output = RustFrontend::new()
            .parse(Path::new("test.rs"), source)
            .unwrap();

        // The real function definition + phantom call target for compute()
        let process_fn = output.nodes.iter()
            .find(|n| n.kind == NodeKind::Function && n.name == "process")
            .expect("process function not found");
        assert_eq!(process_fn.visibility, Visibility::Public);

        // Signature should contain the generics and lifetime
        let sig = process_fn.signature.as_ref().unwrap();
        assert!(sig.contains("<'a, T: Send + Sync>"), "Signature missing generics: {sig}");
        assert!(sig.contains("&'a [T]"), "Signature missing lifetime param: {sig}");
    }
}
