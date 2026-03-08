//! Graph diffing for CI/CD integration — Breaking Change Guardian.
//!
//! Compares two CodeGraphs and produces a structured diff including
//! breaking changes, complexity changes, and new dead code.

use std::collections::HashMap;

use serde::Serialize;

use crate::gir::{ComplexityMetrics, NodeKind, Visibility};
use crate::graph::CodeGraph;
use crate::symbol_id::SymbolId;

/// A diff entry representing an added or removed symbol.
#[derive(Debug, Clone, Serialize)]
pub struct DiffEntry {
    pub name: String,
    pub kind: NodeKind,
    pub file_path: String,
    pub line: u32,
    pub visibility: Visibility,
}

/// A symbol that exists in both graphs but has changed.
#[derive(Debug, Clone, Serialize)]
pub struct ChangedSymbol {
    pub name: String,
    pub kind: NodeKind,
    pub file_path: String,
    pub line: u32,
    pub changes: Vec<ChangeDetail>,
}

/// What specifically changed about a symbol.
#[derive(Debug, Clone, Serialize)]
pub enum ChangeDetail {
    SignatureChanged {
        old: Option<String>,
        new: Option<String>,
    },
    VisibilityChanged {
        old: Visibility,
        new: Visibility,
    },
    Moved {
        old_file: String,
        old_line: u32,
        new_file: String,
        new_line: u32,
    },
}

/// A breaking change detected between two graph versions.
#[derive(Debug, Clone, Serialize)]
pub struct BreakingChange {
    pub severity: Severity,
    pub description: String,
    pub symbol_name: String,
    pub kind: NodeKind,
    pub file_path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

/// A change in complexity metrics between versions.
#[derive(Debug, Clone, Serialize)]
pub struct ComplexityChange {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub old: ComplexityMetrics,
    pub new: ComplexityMetrics,
    pub cyclomatic_delta: i32,
    pub cognitive_delta: i32,
}

/// The full diff between two graph versions.
#[derive(Debug, Clone, Serialize)]
pub struct GraphDiff {
    pub removed_symbols: Vec<DiffEntry>,
    pub added_symbols: Vec<DiffEntry>,
    pub changed_symbols: Vec<ChangedSymbol>,
    pub breaking_changes: Vec<BreakingChange>,
    pub complexity_changes: Vec<ComplexityChange>,
    pub new_dead_code: Vec<DiffEntry>,
}

/// Compare two CodeGraphs and produce a structured diff.
pub fn diff_graphs(base: &CodeGraph, head: &CodeGraph) -> GraphDiff {
    // Build lookup maps by SymbolId
    let base_map: HashMap<SymbolId, _> = base.all_nodes().map(|n| (n.id, n)).collect();
    let head_map: HashMap<SymbolId, _> = head.all_nodes().map(|n| (n.id, n)).collect();

    // Secondary index for fuzzy matching moved/renamed symbols: (name, kind) -> node
    let base_by_name_kind: HashMap<(&str, NodeKind), Vec<_>> = {
        let mut m: HashMap<(&str, NodeKind), Vec<_>> = HashMap::new();
        for n in base.all_nodes() {
            m.entry((&n.name, n.kind)).or_default().push(n);
        }
        m
    };

    let mut removed_symbols = Vec::new();
    let mut added_symbols = Vec::new();
    let mut changed_symbols = Vec::new();
    let mut breaking_changes = Vec::new();
    let mut complexity_changes = Vec::new();

    // Find removed symbols (in base but not in head)
    for (id, node) in &base_map {
        if !head_map.contains_key(id) {
            // Check if it was moved/renamed (same name+kind exists in head)
            let moved = head
                .find_by_name(&node.name)
                .iter()
                .any(|h| h.kind == node.kind && !base_map.contains_key(&h.id));

            if !moved {
                removed_symbols.push(DiffEntry {
                    name: node.name.clone(),
                    kind: node.kind,
                    file_path: node.file_path.to_string_lossy().into(),
                    line: node.span.start_line,
                    visibility: node.visibility,
                });

                // Check if it's a breaking change (public API removal)
                if is_public_api(node.visibility) && is_api_relevant_kind(node.kind) {
                    breaking_changes.push(BreakingChange {
                        severity: Severity::Error,
                        description: format!(
                            "Removed public {:?} `{}`",
                            node.kind, node.name
                        ),
                        symbol_name: node.name.clone(),
                        kind: node.kind,
                        file_path: node.file_path.to_string_lossy().into(),
                        line: node.span.start_line,
                    });
                }
            }
        }
    }

    // Find added symbols (in head but not in base)
    for (id, node) in &head_map {
        if !base_map.contains_key(id) {
            let moved = base_by_name_kind
                .get(&(node.name.as_str(), node.kind))
                .map_or(false, |base_nodes| {
                    base_nodes.iter().any(|b| !head_map.contains_key(&b.id))
                });

            if !moved {
                added_symbols.push(DiffEntry {
                    name: node.name.clone(),
                    kind: node.kind,
                    file_path: node.file_path.to_string_lossy().into(),
                    line: node.span.start_line,
                    visibility: node.visibility,
                });
            }
        }
    }

    // Find changed symbols (same SymbolId in both graphs)
    for (id, base_node) in &base_map {
        if let Some(head_node) = head_map.get(id) {
            let mut changes = Vec::new();

            // Check signature changes
            if base_node.signature != head_node.signature {
                changes.push(ChangeDetail::SignatureChanged {
                    old: base_node.signature.clone(),
                    new: head_node.signature.clone(),
                });

                if is_public_api(base_node.visibility) && is_api_relevant_kind(base_node.kind) {
                    breaking_changes.push(BreakingChange {
                        severity: Severity::Warning,
                        description: format!(
                            "Signature changed for public {:?} `{}`",
                            base_node.kind, base_node.name
                        ),
                        symbol_name: base_node.name.clone(),
                        kind: base_node.kind,
                        file_path: head_node.file_path.to_string_lossy().into(),
                        line: head_node.span.start_line,
                    });
                }
            }

            // Check visibility narrowing
            if base_node.visibility != head_node.visibility {
                changes.push(ChangeDetail::VisibilityChanged {
                    old: base_node.visibility,
                    new: head_node.visibility,
                });

                if is_public_api(base_node.visibility) && !is_public_api(head_node.visibility) {
                    breaking_changes.push(BreakingChange {
                        severity: Severity::Error,
                        description: format!(
                            "Visibility narrowed for `{}`: {:?} -> {:?}",
                            base_node.name, base_node.visibility, head_node.visibility
                        ),
                        symbol_name: base_node.name.clone(),
                        kind: base_node.kind,
                        file_path: head_node.file_path.to_string_lossy().into(),
                        line: head_node.span.start_line,
                    });
                }
            }

            // Check file/line moves
            if base_node.file_path != head_node.file_path
                || base_node.span.start_line != head_node.span.start_line
            {
                changes.push(ChangeDetail::Moved {
                    old_file: base_node.file_path.to_string_lossy().into(),
                    old_line: base_node.span.start_line,
                    new_file: head_node.file_path.to_string_lossy().into(),
                    new_line: head_node.span.start_line,
                });
            }

            // Complexity changes
            if let (Some(old_cx), Some(new_cx)) =
                (&base_node.complexity, &head_node.complexity)
            {
                let cyc_delta = new_cx.cyclomatic as i32 - old_cx.cyclomatic as i32;
                let cog_delta = new_cx.cognitive as i32 - old_cx.cognitive as i32;

                if cyc_delta != 0 || cog_delta != 0 {
                    complexity_changes.push(ComplexityChange {
                        name: head_node.name.clone(),
                        file_path: head_node.file_path.to_string_lossy().into(),
                        line: head_node.span.start_line,
                        old: *old_cx,
                        new: *new_cx,
                        cyclomatic_delta: cyc_delta,
                        cognitive_delta: cog_delta,
                    });
                }
            }

            if !changes.is_empty() {
                changed_symbols.push(ChangedSymbol {
                    name: head_node.name.clone(),
                    kind: head_node.kind,
                    file_path: head_node.file_path.to_string_lossy().into(),
                    line: head_node.span.start_line,
                    changes,
                });
            }
        }
    }

    // Detect new dead code in head (callable symbols with no callers that weren't in base)
    let new_dead_code: Vec<DiffEntry> = head
        .all_nodes()
        .filter(|n| n.kind.is_callable())
        .filter(|n| !base_map.contains_key(&n.id))
        .filter(|n| {
            head.callers(n.id).is_empty()
                && n.name != "main"
                && n.name != "__init__"
                && !n.name.starts_with("test_")
        })
        .map(|n| DiffEntry {
            name: n.name.clone(),
            kind: n.kind,
            file_path: n.file_path.to_string_lossy().into(),
            line: n.span.start_line,
            visibility: n.visibility,
        })
        .collect();

    GraphDiff {
        removed_symbols,
        added_symbols,
        changed_symbols,
        breaking_changes,
        complexity_changes,
        new_dead_code,
    }
}

/// Format the diff as human-readable text for CLI output.
pub fn format_diff_text(diff: &GraphDiff) -> String {
    let mut out = String::new();

    // Breaking changes first (most important)
    if !diff.breaking_changes.is_empty() {
        out.push_str(&format!(
            "BREAKING CHANGES ({}):\n",
            diff.breaking_changes.len()
        ));
        for bc in &diff.breaking_changes {
            let icon = match bc.severity {
                Severity::Error => "ERROR",
                Severity::Warning => "WARN ",
            };
            out.push_str(&format!(
                "  [{}] {} ({}:{})\n",
                icon, bc.description, bc.file_path, bc.line
            ));
        }
        out.push('\n');
    }

    // Summary
    out.push_str(&format!(
        "Summary: +{} added, -{} removed, ~{} changed\n",
        diff.added_symbols.len(),
        diff.removed_symbols.len(),
        diff.changed_symbols.len(),
    ));

    if !diff.added_symbols.is_empty() {
        out.push_str(&format!("\nAdded ({}):\n", diff.added_symbols.len()));
        for s in &diff.added_symbols {
            out.push_str(&format!(
                "  + {:?} {} ({}:{})\n",
                s.kind, s.name, s.file_path, s.line
            ));
        }
    }

    if !diff.removed_symbols.is_empty() {
        out.push_str(&format!("\nRemoved ({}):\n", diff.removed_symbols.len()));
        for s in &diff.removed_symbols {
            out.push_str(&format!(
                "  - {:?} {} ({}:{})\n",
                s.kind, s.name, s.file_path, s.line
            ));
        }
    }

    if !diff.changed_symbols.is_empty() {
        out.push_str(&format!("\nChanged ({}):\n", diff.changed_symbols.len()));
        for s in &diff.changed_symbols {
            out.push_str(&format!("  ~ {:?} {} ({}:{})\n", s.kind, s.name, s.file_path, s.line));
            for change in &s.changes {
                match change {
                    ChangeDetail::SignatureChanged { old, new } => {
                        out.push_str(&format!(
                            "      signature: {} -> {}\n",
                            old.as_deref().unwrap_or("(none)"),
                            new.as_deref().unwrap_or("(none)")
                        ));
                    }
                    ChangeDetail::VisibilityChanged { old, new } => {
                        out.push_str(&format!("      visibility: {:?} -> {:?}\n", old, new));
                    }
                    ChangeDetail::Moved {
                        old_file,
                        old_line,
                        new_file,
                        new_line,
                    } => {
                        out.push_str(&format!(
                            "      moved: {}:{} -> {}:{}\n",
                            old_file, old_line, new_file, new_line
                        ));
                    }
                }
            }
        }
    }

    if !diff.complexity_changes.is_empty() {
        out.push_str(&format!(
            "\nComplexity changes ({}):\n",
            diff.complexity_changes.len()
        ));
        for c in &diff.complexity_changes {
            let cyc_arrow = if c.cyclomatic_delta > 0 { "+" } else { "" };
            let cog_arrow = if c.cognitive_delta > 0 { "+" } else { "" };
            out.push_str(&format!(
                "  {} ({}:{}): cyclomatic {}{}, cognitive {}{}\n",
                c.name, c.file_path, c.line, cyc_arrow, c.cyclomatic_delta, cog_arrow, c.cognitive_delta
            ));
        }
    }

    if !diff.new_dead_code.is_empty() {
        out.push_str(&format!(
            "\nNew dead code ({}):\n",
            diff.new_dead_code.len()
        ));
        for s in &diff.new_dead_code {
            out.push_str(&format!(
                "  ! {:?} {} ({}:{})\n",
                s.kind, s.name, s.file_path, s.line
            ));
        }
    }

    out
}

fn is_public_api(vis: Visibility) -> bool {
    matches!(vis, Visibility::Public | Visibility::Exported)
}

fn is_api_relevant_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Enum
            | NodeKind::Interface
            | NodeKind::Trait
            | NodeKind::TypeAlias
            | NodeKind::Constant
            | NodeKind::Property
            | NodeKind::Field
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gir::{GirNode, Language, Span};
    use std::path::PathBuf;

    fn make_node(name: &str, kind: NodeKind, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            kind,
            PathBuf::from("test.py"),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    fn make_public_node(name: &str, kind: NodeKind, line: u32) -> GirNode {
        let mut n = make_node(name, kind, line);
        n.visibility = Visibility::Public;
        n
    }

    #[test]
    fn detect_added_and_removed() {
        let mut base = CodeGraph::new();
        let mut head = CodeGraph::new();

        base.add_node(make_node("old_func", NodeKind::Function, 1));
        base.add_node(make_node("shared_func", NodeKind::Function, 10));

        head.add_node(make_node("shared_func", NodeKind::Function, 10));
        head.add_node(make_node("new_func", NodeKind::Function, 20));

        let diff = diff_graphs(&base, &head);

        assert_eq!(diff.removed_symbols.len(), 1);
        assert_eq!(diff.removed_symbols[0].name, "old_func");
        assert_eq!(diff.added_symbols.len(), 1);
        assert_eq!(diff.added_symbols[0].name, "new_func");
    }

    #[test]
    fn detect_breaking_change_removal() {
        let mut base = CodeGraph::new();
        let head = CodeGraph::new();

        base.add_node(make_public_node("public_api", NodeKind::Function, 1));

        let diff = diff_graphs(&base, &head);

        assert_eq!(diff.breaking_changes.len(), 1);
        assert_eq!(diff.breaking_changes[0].severity, Severity::Error);
        assert!(diff.breaking_changes[0].description.contains("Removed"));
    }

    #[test]
    fn detect_signature_change() {
        let mut base = CodeGraph::new();
        let mut head = CodeGraph::new();

        let mut n1 = make_public_node("my_func", NodeKind::Function, 1);
        n1.signature = Some("fn my_func(a: i32)".into());

        let mut n2 = make_public_node("my_func", NodeKind::Function, 1);
        n2.signature = Some("fn my_func(a: i32, b: i32)".into());

        base.add_node(n1);
        head.add_node(n2);

        let diff = diff_graphs(&base, &head);

        assert_eq!(diff.changed_symbols.len(), 1);
        assert!(diff.breaking_changes.iter().any(|bc| bc.description.contains("Signature changed")));
    }

    #[test]
    fn detect_new_dead_code() {
        let mut base = CodeGraph::new();
        let mut head = CodeGraph::new();

        // Base has a function
        base.add_node(make_node("existing", NodeKind::Function, 1));

        // Head has existing + new uncalled function
        head.add_node(make_node("existing", NodeKind::Function, 1));
        head.add_node(make_node("unused_new", NodeKind::Function, 20));

        let diff = diff_graphs(&base, &head);

        assert_eq!(diff.new_dead_code.len(), 1);
        assert_eq!(diff.new_dead_code[0].name, "unused_new");
    }

    #[test]
    fn no_false_positive_for_test_functions() {
        let mut base = CodeGraph::new();
        let mut head = CodeGraph::new();

        base.add_node(make_node("existing", NodeKind::Function, 1));
        head.add_node(make_node("existing", NodeKind::Function, 1));
        head.add_node(make_node("test_something", NodeKind::Function, 20));

        let diff = diff_graphs(&base, &head);

        // test_ functions should not be flagged as dead code
        assert!(diff.new_dead_code.is_empty());
    }

    #[test]
    fn both_graphs_empty() {
        let base = CodeGraph::new();
        let head = CodeGraph::new();
        let diff = diff_graphs(&base, &head);
        assert!(diff.removed_symbols.is_empty());
        assert!(diff.added_symbols.is_empty());
        assert!(diff.changed_symbols.is_empty());
        assert!(diff.breaking_changes.is_empty());
        assert!(diff.new_dead_code.is_empty());
    }

    #[test]
    fn visibility_change_is_breaking() {
        let mut base = CodeGraph::new();
        let mut head = CodeGraph::new();

        base.add_node(make_public_node("api_func", NodeKind::Function, 1));
        // Same function but now private
        let n = make_node("api_func", NodeKind::Function, 1);
        head.add_node(n);

        let diff = diff_graphs(&base, &head);
        assert!(diff.breaking_changes.iter().any(|bc| bc.description.contains("Visibility")));
    }
}
