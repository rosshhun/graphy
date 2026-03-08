//! Codified context generation.
//!
//! Auto-generates a machine-readable "codebase constitution" from the graph:
//! module boundaries, naming conventions, architectural patterns, entry points,
//! hotspots, and public API surface.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use graphy_core::{CodeGraph, EdgeKind, NodeKind, Visibility};

use crate::flow_detection;

/// Full codebase context output.
#[derive(Debug, Clone, Serialize)]
pub struct CodebaseContext {
    pub project_name: String,
    pub summary: ProjectSummary,
    pub module_map: Vec<ModuleEntry>,
    pub entry_points: Vec<EntryPointInfo>,
    pub architectural_patterns: Vec<ArchPattern>,
    pub naming_conventions: NamingConventions,
    pub hotspots: Vec<HotspotEntry>,
    pub public_api: Vec<ApiSymbol>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub languages: Vec<LanguageCount>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub function_count: usize,
    pub class_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageCount {
    pub language: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleEntry {
    pub directory: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub description: String,
    pub top_symbols: Vec<String>,
    pub depends_on: Vec<String>,
    pub depended_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryPointInfo {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub reachable_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchPattern {
    pub name: String,
    pub description: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamingConventions {
    pub dominant_case: String,
    pub function_prefixes: Vec<String>,
    pub class_prefixes: Vec<String>,
    pub test_pattern: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotspotEntry {
    pub name: String,
    pub file_path: String,
    pub line: u32,
    pub cyclomatic: u32,
    pub cognitive: u32,
    pub caller_count: usize,
    pub risk_score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub signature: Option<String>,
}

/// Generate a comprehensive codebase context from the graph.
pub fn generate_context(graph: &mut CodeGraph, root: &Path) -> CodebaseContext {
    let project_name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    CodebaseContext {
        project_name,
        summary: build_summary(graph),
        module_map: build_module_map(graph, root),
        entry_points: detect_entry_points(graph),
        architectural_patterns: detect_arch_patterns(graph),
        naming_conventions: analyze_naming(graph),
        hotspots: find_hotspots(graph),
        public_api: extract_public_api(graph),
    }
}

fn build_summary(graph: &CodeGraph) -> ProjectSummary {
    let mut lang_counts: HashMap<String, usize> = HashMap::new();
    for node in graph.all_nodes() {
        *lang_counts
            .entry(format!("{:?}", node.language))
            .or_default() += 1;
    }

    let mut languages: Vec<LanguageCount> = lang_counts
        .into_iter()
        .map(|(language, count)| LanguageCount { language, count })
        .collect();
    languages.sort_by(|a, b| b.count.cmp(&a.count));

    ProjectSummary {
        languages,
        file_count: graph.find_by_kind(NodeKind::File).len(),
        symbol_count: graph.node_count(),
        edge_count: graph.edge_count(),
        function_count: graph.find_by_kind(NodeKind::Function).len(),
        class_count: graph.find_by_kind(NodeKind::Class).len(),
    }
}

fn build_module_map(graph: &CodeGraph, root: &Path) -> Vec<ModuleEntry> {
    let mut dir_symbols: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut dir_files: HashMap<PathBuf, usize> = HashMap::new();

    for node in graph.all_nodes() {
        if node.kind == NodeKind::File {
            if let Some(parent) = node.file_path.parent() {
                *dir_files.entry(parent.to_path_buf()).or_default() += 1;
            }
            continue;
        }
        if let Some(parent) = node.file_path.parent() {
            dir_symbols
                .entry(parent.to_path_buf())
                .or_default()
                .push(node.name.clone());
        }
    }

    // Build import-based dependency graph between directories
    let mut dir_deps: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for node in graph.all_nodes() {
        if node.kind == NodeKind::Import {
            let targets = graph.outgoing(node.id, EdgeKind::ImportsFrom);
            for target in &targets {
                if let (Some(src_dir), Some(tgt_dir)) =
                    (node.file_path.parent(), target.file_path.parent())
                {
                    if src_dir != tgt_dir {
                        dir_deps
                            .entry(src_dir.to_path_buf())
                            .or_default()
                            .push(tgt_dir.to_path_buf());
                    }
                }
            }
        }
    }

    let mut modules: Vec<ModuleEntry> = dir_symbols
        .iter()
        .map(|(dir, symbols)| {
            let rel_dir = dir
                .strip_prefix(root)
                .unwrap_or(dir)
                .to_string_lossy()
                .to_string();

            // Auto-label: find most common terms in symbol names
            let description = auto_label_module(symbols);

            let top_symbols: Vec<String> = {
                let mut s = symbols.clone();
                s.sort();
                s.dedup();
                s.truncate(10);
                s
            };

            let depends_on: Vec<String> = dir_deps
                .get(dir)
                .map(|deps| {
                    let mut d: Vec<String> = deps
                        .iter()
                        .map(|d| {
                            d.strip_prefix(root)
                                .unwrap_or(d)
                                .to_string_lossy()
                                .to_string()
                        })
                        .collect();
                    d.sort();
                    d.dedup();
                    d
                })
                .unwrap_or_default();

            // Find who depends on this directory
            let depended_by: Vec<String> = dir_deps
                .iter()
                .filter(|(_, targets)| targets.contains(dir))
                .map(|(src, _)| {
                    src.strip_prefix(root)
                        .unwrap_or(src)
                        .to_string_lossy()
                        .to_string()
                })
                .collect();

            ModuleEntry {
                directory: rel_dir,
                file_count: *dir_files.get(dir).unwrap_or(&0),
                symbol_count: symbols.len(),
                description,
                top_symbols,
                depends_on,
                depended_by,
            }
        })
        .collect();

    modules.sort_by(|a, b| b.symbol_count.cmp(&a.symbol_count));
    modules
}

fn auto_label_module(symbols: &[String]) -> String {
    let mut term_freq: HashMap<String, usize> = HashMap::new();
    for name in symbols {
        // Split camelCase and snake_case
        for word in split_identifier(name) {
            let lower = word.to_lowercase();
            if lower.len() > 2 {
                *term_freq.entry(lower).or_default() += 1;
            }
        }
    }

    let mut terms: Vec<(String, usize)> = term_freq.into_iter().collect();
    terms.sort_by(|a, b| b.1.cmp(&a.1));

    terms
        .iter()
        .take(3)
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_identifier(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if ch.is_uppercase() && !current.is_empty() {
            words.push(current.clone());
            current.clear();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn detect_entry_points(graph: &mut CodeGraph) -> Vec<EntryPointInfo> {
    let flows = flow_detection::detect_flows(graph);

    flows
        .iter()
        .map(|f| EntryPointInfo {
            name: f.entry_name.clone(),
            kind: format!("{:?}", f.flow_kind),
            file_path: f.entry_file.to_string_lossy().into(),
            line: graph
                .get_node(f.entry_point)
                .map(|n| n.span.start_line)
                .unwrap_or(0),
            reachable_count: f.reachable.len(),
        })
        .collect()
}

fn detect_arch_patterns(graph: &CodeGraph) -> Vec<ArchPattern> {
    let mut patterns = Vec::new();

    // Detect MVC/MVT pattern
    let dirs: Vec<String> = graph
        .all_nodes()
        .filter(|n| n.kind == NodeKind::File)
        .filter_map(|n| {
            n.file_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_lowercase())
        })
        .collect();

    let has_models = dirs.iter().any(|d| d.contains("model"));
    let has_views = dirs.iter().any(|d| d.contains("view"));
    let has_controllers = dirs.iter().any(|d| d.contains("controller"));
    let has_templates = dirs.iter().any(|d| d.contains("template"));
    let has_handlers = dirs.iter().any(|d| d.contains("handler"));
    let has_services = dirs.iter().any(|d| d.contains("service"));

    if has_models && has_views && has_controllers {
        patterns.push(ArchPattern {
            name: "MVC".into(),
            description: "Model-View-Controller architecture detected".into(),
            evidence: vec![
                "models/ directory".into(),
                "views/ directory".into(),
                "controllers/ directory".into(),
            ],
        });
    } else if has_models && has_views && has_templates {
        patterns.push(ArchPattern {
            name: "MVT".into(),
            description: "Model-View-Template (Django-style) architecture detected".into(),
            evidence: vec![
                "models/ directory".into(),
                "views/ directory".into(),
                "templates/ directory".into(),
            ],
        });
    }

    if has_handlers && has_services {
        patterns.push(ArchPattern {
            name: "Handler-Service".into(),
            description: "Handler/Service layered architecture detected".into(),
            evidence: vec![
                "handlers/ directory".into(),
                "services/ directory".into(),
            ],
        });
    }

    // Detect event-driven patterns
    let listener_count = graph
        .all_nodes()
        .filter(|n| {
            n.name.contains("listener")
                || n.name.contains("handler")
                || n.name.contains("on_")
                || n.name.contains("emit")
        })
        .count();

    if listener_count > 5 {
        patterns.push(ArchPattern {
            name: "Event-Driven".into(),
            description: "Event-driven patterns detected (listeners/handlers/emitters)".into(),
            evidence: vec![format!("{} event-related symbols found", listener_count)],
        });
    }

    patterns
}

fn analyze_naming(graph: &CodeGraph) -> NamingConventions {
    let mut snake_count = 0usize;
    let mut camel_count = 0usize;
    let mut pascal_count = 0usize;
    let mut func_prefixes: HashMap<String, usize> = HashMap::new();
    let mut class_prefixes: HashMap<String, usize> = HashMap::new();

    for node in graph.all_nodes() {
        match node.kind {
            NodeKind::Function | NodeKind::Method => {
                if node.name.contains('_') {
                    snake_count += 1;
                } else if node.name.chars().next().map_or(false, |c| c.is_lowercase()) {
                    camel_count += 1;
                }

                // Extract prefix (first word)
                let prefix = split_identifier(&node.name)
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .to_lowercase();
                if prefix.len() > 1 {
                    *func_prefixes.entry(prefix).or_default() += 1;
                }
            }
            NodeKind::Class | NodeKind::Struct => {
                if node.name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    pascal_count += 1;
                }

                let prefix = split_identifier(&node.name)
                    .first()
                    .cloned()
                    .unwrap_or_default();
                if prefix.len() > 1 {
                    *class_prefixes.entry(prefix).or_default() += 1;
                }
            }
            _ => {}
        }
    }

    let dominant_case = if snake_count > camel_count && snake_count > pascal_count {
        "snake_case"
    } else if camel_count > snake_count {
        "camelCase"
    } else {
        "PascalCase"
    }
    .to_string();

    let mut fp: Vec<(String, usize)> = func_prefixes.into_iter().collect();
    fp.sort_by(|a, b| b.1.cmp(&a.1));
    let function_prefixes: Vec<String> = fp.iter().take(5).map(|(k, _)| k.clone()).collect();

    let mut cp: Vec<(String, usize)> = class_prefixes.into_iter().collect();
    cp.sort_by(|a, b| b.1.cmp(&a.1));
    let class_prefixes: Vec<String> = cp.iter().take(5).map(|(k, _)| k.clone()).collect();

    let test_functions: Vec<_> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable() && n.name.starts_with("test"))
        .collect();
    let test_pattern = if test_functions
        .iter()
        .all(|n| n.name.starts_with("test_"))
    {
        "test_*".to_string()
    } else if test_functions
        .iter()
        .all(|n| n.name.starts_with("Test"))
    {
        "Test*".to_string()
    } else {
        "mixed".to_string()
    };

    NamingConventions {
        dominant_case,
        function_prefixes,
        class_prefixes,
        test_pattern,
    }
}

fn find_hotspots(graph: &CodeGraph) -> Vec<HotspotEntry> {
    let mut hotspots: Vec<HotspotEntry> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable())
        .filter_map(|n| {
            let cx = n.complexity.as_ref()?;
            let caller_count = graph.callers(n.id).len();
            let complexity = cx.cyclomatic as f64;
            let risk_score = complexity * (1.0 + caller_count as f64 * 0.5);

            Some(HotspotEntry {
                name: n.name.clone(),
                file_path: n.file_path.to_string_lossy().into(),
                line: n.span.start_line,
                cyclomatic: cx.cyclomatic,
                cognitive: cx.cognitive,
                caller_count,
                risk_score,
            })
        })
        .collect();

    hotspots.sort_by(|a, b| b.risk_score.partial_cmp(&a.risk_score).unwrap_or(std::cmp::Ordering::Equal));
    hotspots.truncate(20);
    hotspots
}

fn extract_public_api(graph: &CodeGraph) -> Vec<ApiSymbol> {
    let mut api: Vec<ApiSymbol> = graph
        .all_nodes()
        .filter(|n| {
            (n.kind.is_callable() || n.kind.is_type_def())
                && matches!(n.visibility, Visibility::Public | Visibility::Exported)
        })
        .map(|n| ApiSymbol {
            name: n.name.clone(),
            kind: format!("{:?}", n.kind),
            file_path: n.file_path.to_string_lossy().into(),
            line: n.span.start_line,
            signature: n.signature.clone(),
        })
        .collect();

    api.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    api
}

/// Format the context as Markdown.
pub fn format_as_markdown(ctx: &CodebaseContext) -> String {
    let mut out = String::new();

    out.push_str(&format!("# Codebase Context: {}\n\n", ctx.project_name));

    // Summary
    out.push_str("## Summary\n\n");
    out.push_str(&format!("- **Files**: {}\n", ctx.summary.file_count));
    out.push_str(&format!("- **Symbols**: {}\n", ctx.summary.symbol_count));
    out.push_str(&format!("- **Edges**: {}\n", ctx.summary.edge_count));
    out.push_str(&format!("- **Functions**: {}\n", ctx.summary.function_count));
    out.push_str(&format!("- **Classes**: {}\n", ctx.summary.class_count));
    out.push_str("- **Languages**: ");
    let langs: Vec<String> = ctx
        .summary
        .languages
        .iter()
        .map(|l| format!("{} ({})", l.language, l.count))
        .collect();
    out.push_str(&langs.join(", "));
    out.push_str("\n\n");

    // Module map
    if !ctx.module_map.is_empty() {
        out.push_str("## Module Map\n\n");
        for m in &ctx.module_map {
            out.push_str(&format!(
                "### `{}`\n- {} files, {} symbols\n- Keywords: {}\n",
                m.directory, m.file_count, m.symbol_count, m.description
            ));
            if !m.depends_on.is_empty() {
                out.push_str(&format!("- Depends on: {}\n", m.depends_on.join(", ")));
            }
            if !m.depended_by.is_empty() {
                out.push_str(&format!("- Depended by: {}\n", m.depended_by.join(", ")));
            }
            out.push('\n');
        }
    }

    // Entry points
    if !ctx.entry_points.is_empty() {
        out.push_str("## Entry Points\n\n");
        for ep in &ctx.entry_points {
            out.push_str(&format!(
                "- `{}` ({}) at {}:{} — reaches {} symbols\n",
                ep.name, ep.kind, ep.file_path, ep.line, ep.reachable_count
            ));
        }
        out.push('\n');
    }

    // Architecture
    if !ctx.architectural_patterns.is_empty() {
        out.push_str("## Architectural Patterns\n\n");
        for ap in &ctx.architectural_patterns {
            out.push_str(&format!("### {}\n{}\n", ap.name, ap.description));
            for e in &ap.evidence {
                out.push_str(&format!("- {}\n", e));
            }
            out.push('\n');
        }
    }

    // Naming conventions
    out.push_str("## Naming Conventions\n\n");
    out.push_str(&format!(
        "- Dominant case: **{}**\n",
        ctx.naming_conventions.dominant_case
    ));
    out.push_str(&format!(
        "- Test pattern: `{}`\n",
        ctx.naming_conventions.test_pattern
    ));
    if !ctx.naming_conventions.function_prefixes.is_empty() {
        out.push_str(&format!(
            "- Common function prefixes: {}\n",
            ctx.naming_conventions.function_prefixes.join(", ")
        ));
    }
    out.push('\n');

    // Hotspots
    if !ctx.hotspots.is_empty() {
        out.push_str("## Hotspots (Top Risk)\n\n");
        out.push_str("| Risk | Symbol | Complexity | Callers | Location |\n");
        out.push_str("|------|--------|-----------|---------|----------|\n");
        for h in ctx.hotspots.iter().take(10) {
            out.push_str(&format!(
                "| {:.1} | `{}` | cyc={} cog={} | {} | {}:{} |\n",
                h.risk_score, h.name, h.cyclomatic, h.cognitive, h.caller_count, h.file_path, h.line
            ));
        }
        out.push('\n');
    }

    // Public API
    if !ctx.public_api.is_empty() {
        out.push_str("## Public API\n\n");
        for sym in ctx.public_api.iter().take(30) {
            out.push_str(&format!("- {} `{}`", sym.kind, sym.name));
            if let Some(sig) = &sym.signature {
                out.push_str(&format!(" — `{}`", sig));
            }
            out.push_str(&format!(" ({}:{})\n", sym.file_path, sym.line));
        }
        if ctx.public_api.len() > 30 {
            out.push_str(&format!("... and {} more\n", ctx.public_api.len() - 30));
        }
    }

    out
}

/// Format the context as JSON.
pub fn format_as_json(ctx: &CodebaseContext) -> Value {
    serde_json::to_value(ctx).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirEdge, GirNode, Language, Span, SymbolId, Visibility};

    fn make_fn(name: &str, file: &str, line: u32, vis: Visibility) -> GirNode {
        let mut node = GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        );
        node.visibility = vis;
        node
    }

    fn make_file(name: &str) -> GirNode {
        let path = PathBuf::from(name);
        GirNode {
            id: SymbolId::new(&path, name, NodeKind::File, 0),
            name: name.to_string(),
            kind: NodeKind::File,
            file_path: path,
            span: Span::new(0, 0, 100, 0),
            visibility: Visibility::Public,
            language: Language::Python,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        }
    }

    #[test]
    fn generate_context_empty_graph() {
        let mut g = CodeGraph::new();
        let ctx = generate_context(&mut g, Path::new("/project"));
        assert_eq!(ctx.summary.file_count, 0);
        assert_eq!(ctx.summary.symbol_count, 0);
        assert!(ctx.module_map.is_empty());
        assert!(ctx.entry_points.is_empty());
        assert!(ctx.hotspots.is_empty());
        assert!(ctx.public_api.is_empty());
    }

    #[test]
    fn generate_context_with_functions() {
        let mut g = CodeGraph::new();
        let file = make_file("src/app.py");
        let file_id = file.id;
        g.add_node(file);

        let f1 = make_fn("main", "src/app.py", 1, Visibility::Public);
        let f1_id = f1.id;
        g.add_node(f1);
        g.add_edge(file_id, f1_id, GirEdge::new(EdgeKind::Contains));

        let f2 = make_fn("helper", "src/app.py", 10, Visibility::Internal);
        let f2_id = f2.id;
        g.add_node(f2);
        g.add_edge(file_id, f2_id, GirEdge::new(EdgeKind::Contains));

        let ctx = generate_context(&mut g, Path::new("/project"));
        assert!(ctx.summary.function_count >= 2);
        // Public API should include the public function
        assert!(ctx.public_api.iter().any(|s| s.name == "main"));
    }

    #[test]
    fn format_as_markdown_not_empty() {
        let ctx = CodebaseContext {
            project_name: "test".into(),
            summary: ProjectSummary {
                languages: vec![LanguageCount { language: "Python".into(), count: 5 }],
                file_count: 5,
                symbol_count: 20,
                edge_count: 15,
                function_count: 10,
                class_count: 3,
            },
            module_map: vec![],
            entry_points: vec![],
            architectural_patterns: vec![],
            naming_conventions: NamingConventions {
                dominant_case: "snake_case".into(),
                function_prefixes: vec![],
                class_prefixes: vec![],
                test_pattern: String::new(),
            },
            hotspots: vec![],
            public_api: vec![],
        };
        let md = format_as_markdown(&ctx);
        assert!(md.contains("# Codebase Context: test"));
        assert!(md.contains("Python"));
    }

    #[test]
    fn format_as_json_round_trip() {
        let ctx = CodebaseContext {
            project_name: "test".into(),
            summary: ProjectSummary {
                languages: vec![],
                file_count: 0,
                symbol_count: 0,
                edge_count: 0,
                function_count: 0,
                class_count: 0,
            },
            module_map: vec![],
            entry_points: vec![],
            architectural_patterns: vec![],
            naming_conventions: NamingConventions {
                dominant_case: "snake_case".into(),
                function_prefixes: vec![],
                class_prefixes: vec![],
                test_pattern: String::new(),
            },
            hotspots: vec![],
            public_api: vec![],
        };
        let json = format_as_json(&ctx);
        assert!(json.is_object());
        assert_eq!(json["project_name"], "test");
    }
}
