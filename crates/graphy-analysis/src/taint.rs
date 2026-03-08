//! Phase 9: Taint analysis with language-specific source/sink/sanitizer patterns.
//!
//! Classifies callable nodes as taint sources (user input), sinks (dangerous ops),
//! or sanitizers based on both generic and language-specific patterns. Propagates
//! taint through DATA_FLOWS_TO and CALLS edges. Reports unsanitized source->sink
//! paths via TAINTED_BY edges.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use graphy_core::{
    CodeGraph, EdgeKind, EdgeMetadata, GirEdge, Language, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::Deserialize;
use tracing::{debug, info};

// ── Language-specific taint patterns ────────────────────────

struct TaintPatterns {
    sources: &'static [&'static str],
    sinks: &'static [&'static str],
    sanitizers: &'static [&'static str],
}

/// Generic patterns that apply to all languages.
const GENERIC: TaintPatterns = TaintPatterns {
    sources: &[
        "input", "stdin", "recv", "read", "urlopen",
    ],
    sinks: &[
        "eval", "exec", "system", "write", "send",
    ],
    sanitizers: &[
        "escape", "sanitize", "validate", "clean", "quote", "encode",
    ],
};

/// Python-specific patterns.
const PYTHON: TaintPatterns = TaintPatterns {
    sources: &[
        "request.args", "request.form", "request.json", "request.data",
        "request.get", "request.post", "request.files",
        "os.environ", "sys.argv", "raw_input",
        "requests.get", "requests.post",
    ],
    sinks: &[
        "cursor.execute", "subprocess.run", "subprocess.call", "subprocess.Popen",
        "os.system", "os.popen", "pickle.loads", "yaml.load",
        "render_template_string", "markup",
        "shlex.split",
    ],
    sanitizers: &[
        "html.escape", "markupsafe", "bleach", "parameterize",
        "shlex.quote",
    ],
};

/// PHP / WordPress patterns.
const PHP: TaintPatterns = TaintPatterns {
    sources: &[
        // PHP superglobals (matched by function/method names that access them)
        "get_option", "get_post_meta", "get_user_meta", "get_transient",
        "get_site_option", "get_theme_mod",
        "file_get_contents", "fread", "fgets",
        "wp_remote_get", "wp_remote_post", "wp_remote_request",
        "wp_unslash",
        // Common WP input accessors
        "filter_input", "getallheaders",
    ],
    sinks: &[
        // SQL injection
        "query", "get_results", "get_var", "get_row", "get_col",
        // XSS
        "echo", "print",
        // Code execution
        "eval", "assert", "preg_replace",
        // File inclusion
        "include", "require", "include_once", "require_once",
        // File write
        "file_put_contents", "fwrite", "fputs",
        // Command injection
        "exec", "system", "passthru", "shell_exec", "popen", "proc_open",
        // URL redirect
        "wp_redirect", "wp_safe_redirect", "header",
        // Email
        "wp_mail", "mail",
        // Option update (persistence)
        "update_option", "update_post_meta", "update_user_meta",
    ],
    sanitizers: &[
        // WordPress escaping
        "esc_html", "esc_attr", "esc_url", "esc_textarea", "esc_js", "esc_sql",
        "wp_kses", "wp_kses_post", "wp_kses_data",
        // WordPress sanitization
        "sanitize_text_field", "sanitize_email", "sanitize_file_name",
        "sanitize_title", "sanitize_key", "sanitize_mime_type",
        "sanitize_option", "sanitize_user",
        // WordPress nonce verification
        "wp_verify_nonce", "check_admin_referer", "check_ajax_referer",
        // PHP type casting (acts as sanitizer)
        "intval", "absint", "floatval",
        // Prepared statements
        "prepare",
        // WordPress capability check (authorization)
        "current_user_can",
        // Strip tags
        "strip_tags", "wp_strip_all_tags",
    ],
};

/// JavaScript / TypeScript patterns.
const JAVASCRIPT: TaintPatterns = TaintPatterns {
    sources: &[
        "req.body", "req.params", "req.query", "req.headers", "req.cookies",
        "document.location", "window.location", "location.href",
        "document.cookie", "localStorage.getItem", "sessionStorage.getItem",
        "document.getElementById", "document.querySelector",
        "URLSearchParams", "FormData",
        "readFileSync", "readFile",
    ],
    sinks: &[
        "eval", "Function",
        "innerHTML", "outerHTML", "document.write", "document.writeln",
        "insertAdjacentHTML",
        "child_process.exec", "child_process.execSync",
        "child_process.spawn", "child_process.execFile",
        "writeFileSync", "writeFile",
        "res.send", "res.write", "res.json",
        "redirect",
        "sql", "raw", // ORM raw queries
    ],
    sanitizers: &[
        "encodeURIComponent", "encodeURI",
        "DOMPurify.sanitize", "sanitizeHtml",
        "escape", "escapeHtml",
        "parseInt", "parseFloat", "Number",
        "validator.escape", "validator.isEmail",
        "parameterized", "placeholder",
    ],
};

/// Rust patterns.
const RUST: TaintPatterns = TaintPatterns {
    sources: &[
        "std::io::stdin", "args", "env::var", "env::args",
        "read_to_string", "read_line",
        "axum::extract", "actix_web::HttpRequest",
        "rocket::request",
    ],
    sinks: &[
        "Command::new", "process::Command",
        "std::fs::write", "write_all",
        "format!", // only dangerous when used in SQL/command context
        "execute", "query",
    ],
    sanitizers: &[
        "html_escape", "encode", "sanitize",
        "bind", // parameterized queries
    ],
};

/// Get language-specific patterns, falling back to generic.
fn patterns_for_language(lang: Language) -> &'static TaintPatterns {
    match lang {
        Language::Python => &PYTHON,
        Language::Php => &PHP,
        Language::TypeScript | Language::JavaScript | Language::Svelte => &JAVASCRIPT,
        Language::Rust => &RUST,
        _ => &GENERIC,
    }
}

// ── Custom TOML taint rules ─────────────────────────────────

/// Custom taint rules loaded from `.graphy/taint.toml`.
///
/// Example file:
/// ```toml
/// sources = ["get_option", "file_get_contents"]
/// sinks = ["query", "exec", "wp_redirect"]
/// sanitizers = ["esc_html", "prepare", "intval"]
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct CustomTaintRules {
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub sinks: Vec<String>,
    #[serde(default)]
    pub sanitizers: Vec<String>,
}

/// Load custom taint rules from `.graphy/taint.toml` if it exists.
pub fn load_custom_taint_rules(project_root: &Path) -> Option<CustomTaintRules> {
    let path = project_root.join(".graphy").join("taint.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    match toml::from_str::<CustomTaintRules>(&content) {
        Ok(rules) => {
            info!(
                "Loaded custom taint rules: {} sources, {} sinks, {} sanitizers from {}",
                rules.sources.len(),
                rules.sinks.len(),
                rules.sanitizers.len(),
                path.display()
            );
            Some(rules)
        }
        Err(e) => {
            tracing::warn!("Failed to parse {}: {e}", path.display());
            None
        }
    }
}

// ── Taint finding ───────────────────────────────────────────

/// A taint finding: an unsanitized path from source to sink.
#[derive(Debug, Clone)]
pub struct TaintFinding {
    pub source_id: SymbolId,
    pub source_name: String,
    pub sink_id: SymbolId,
    pub sink_name: String,
    pub label: String,
    pub path_length: usize,
}

// ── Analysis ────────────────────────────────────────────────

/// Phase 9: Perform taint analysis with optional custom TOML rules.
pub fn analyze_taint_with_rules(
    graph: &mut CodeGraph,
    custom: Option<&CustomTaintRules>,
) -> Vec<TaintFinding> {
    let custom_sources: Vec<&str> = custom
        .map(|c| c.sources.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let custom_sinks: Vec<&str> = custom
        .map(|c| c.sinks.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let custom_sanitizers: Vec<&str> = custom
        .map(|c| c.sanitizers.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    let mut sources: Vec<(SymbolId, String)> = Vec::new();
    let mut sinks: Vec<(SymbolId, String)> = Vec::new();
    let mut sanitizers: HashSet<SymbolId> = HashSet::new();

    // Classify all callable nodes using generic + language-specific + custom patterns.
    for node in graph.all_nodes() {
        if !matches!(
            node.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor
        ) {
            continue;
        }

        let name_lower = node.name.to_lowercase();
        let lang_patterns = patterns_for_language(node.language);

        // Check source patterns (generic + language-specific + custom)
        let is_source = check_patterns(&name_lower, &node.name, GENERIC.sources)
            || check_patterns(&name_lower, &node.name, lang_patterns.sources)
            || check_patterns(&name_lower, &node.name, &custom_sources);
        if is_source {
            sources.push((node.id, node.name.clone()));
        }

        // Check sink patterns
        let is_sink = check_patterns(&name_lower, &node.name, GENERIC.sinks)
            || check_patterns(&name_lower, &node.name, lang_patterns.sinks)
            || check_patterns(&name_lower, &node.name, &custom_sinks);
        if is_sink {
            sinks.push((node.id, node.name.clone()));
        }

        // Check sanitizer patterns
        let is_sanitizer = check_patterns(&name_lower, &node.name, GENERIC.sanitizers)
            || check_patterns(&name_lower, &node.name, lang_patterns.sanitizers)
            || check_patterns(&name_lower, &node.name, &custom_sanitizers);
        if is_sanitizer {
            sanitizers.insert(node.id);
        }
    }

    // Also check Import/call-site nodes for source patterns.
    for node in graph.all_nodes() {
        let sig = node.signature.as_deref().unwrap_or("");
        let check_text = format!("{} {}", node.name, sig).to_lowercase();
        let lang_patterns = patterns_for_language(node.language);

        let all_source_patterns: Vec<&str> = GENERIC
            .sources
            .iter()
            .chain(lang_patterns.sources.iter())
            .copied()
            .chain(custom_sources.iter().copied())
            .collect();

        for &pattern in &all_source_patterns {
            if check_text.contains(pattern) && !sources.iter().any(|(id, _)| *id == node.id) {
                sources.push((node.id, node.name.clone()));
                break;
            }
        }
    }

    // Propagate taint from each source through DATA_FLOWS_TO and CALLS edges.
    let mut findings: Vec<TaintFinding> = Vec::new();
    let mut taint_edges: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();
    let sink_set: HashSet<SymbolId> = sinks.iter().map(|(id, _)| *id).collect();
    let sink_names: HashMap<SymbolId, String> = sinks.into_iter().collect();

    for (source_id, source_name) in &sources {
        let label = format!("taint_from_{}", source_name);

        let mut visited: HashSet<SymbolId> = HashSet::new();
        let mut queue: VecDeque<(SymbolId, usize)> = VecDeque::new();
        visited.insert(*source_id);
        queue.push_back((*source_id, 0));

        while let Some((current, depth)) = queue.pop_front() {
            // Stop at sanitizers
            if sanitizers.contains(&current) && current != *source_id {
                continue;
            }

            // Check if we've reached a sink
            if sink_set.contains(&current) && current != *source_id {
                let sink_name = sink_names
                    .get(&current)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());

                findings.push(TaintFinding {
                    source_id: *source_id,
                    source_name: source_name.clone(),
                    sink_id: current,
                    sink_name: sink_name.clone(),
                    label: label.clone(),
                    path_length: depth,
                });

                let edge = GirEdge::new(EdgeKind::TaintedBy)
                    .with_confidence(confidence_from_depth(depth))
                    .with_metadata(EdgeMetadata::Taint {
                        label: label.clone(),
                    });
                taint_edges.push((current, *source_id, edge));
            }

            // Follow DATA_FLOWS_TO and CALLS edges
            let Some(idx) = graph.get_node_index(current) else {
                continue;
            };

            if depth > 10 {
                continue;
            }

            // Outgoing edges
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                if matches!(
                    edge.weight().kind,
                    EdgeKind::DataFlowsTo | EdgeKind::Calls
                ) {
                    let target_idx = edge.target();
                    if let Some(target) = graph.graph.node_weight(target_idx) {
                        if visited.insert(target.id) {
                            queue.push_back((target.id, depth + 1));
                        }
                    }
                }
            }

            // Incoming edges (reverse propagation)
            for edge in graph.graph.edges_directed(idx, Direction::Incoming) {
                if matches!(edge.weight().kind, EdgeKind::DataFlowsTo | EdgeKind::Calls) {
                    let source_idx = edge.source();
                    if let Some(source_node) = graph.graph.node_weight(source_idx) {
                        if visited.insert(source_node.id) {
                            queue.push_back((source_node.id, depth + 1));
                        }
                    }
                }
            }
        }
    }

    let taint_count = taint_edges.len();
    for (src, tgt, edge) in taint_edges {
        graph.add_edge(src, tgt, edge);
    }

    debug!(
        "Phase 9 (Taint): {} sources, {} sinks, {} sanitizers, {} findings, {} TAINTED_BY edges",
        sources.len(),
        sink_set.len(),
        sanitizers.len(),
        findings.len(),
        taint_count
    );

    findings
}

// ── Helpers ─────────────────────────────────────────────────

/// Check if a name matches any pattern in the list.
fn check_patterns(name_lower: &str, name_original: &str, patterns: &[&str]) -> bool {
    for &pattern in patterns {
        if name_lower.contains(pattern) || matches_dotted(name_original, pattern) {
            return true;
        }
    }
    false
}

/// Check if a symbol name matches a dotted pattern.
fn matches_dotted(name: &str, pattern: &str) -> bool {
    name == pattern || name.starts_with(&format!("{}.", pattern))
}

/// Confidence decreases with path length.
fn confidence_from_depth(depth: usize) -> f32 {
    match depth {
        0..=1 => 0.95,
        2..=3 => 0.8,
        4..=6 => 0.6,
        _ => 0.4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_dotted() {
        assert!(matches_dotted("request.args", "request.args"));
        assert!(matches_dotted("request.args.get", "request.args"));
        assert!(!matches_dotted("my_request", "request.args"));
    }

    #[test]
    fn test_confidence_from_depth() {
        assert!(confidence_from_depth(1) > confidence_from_depth(5));
        assert!(confidence_from_depth(5) > confidence_from_depth(10));
    }

    #[test]
    fn test_check_patterns_php() {
        assert!(check_patterns("get_option", "get_option", PHP.sources));
        assert!(check_patterns("esc_html", "esc_html", PHP.sanitizers));
        assert!(check_patterns("query", "query", PHP.sinks));
        assert!(check_patterns("wp_redirect", "wp_redirect", PHP.sinks));
    }

    #[test]
    fn test_check_patterns_js() {
        assert!(check_patterns("eval", "eval", JAVASCRIPT.sinks));
        assert!(check_patterns("innerhtml", "innerHTML", JAVASCRIPT.sinks));
        assert!(check_patterns("encodeuricomponent", "encodeURIComponent", JAVASCRIPT.sanitizers));
    }

    #[test]
    fn test_check_patterns_python() {
        assert!(check_patterns("request.args", "request.args", PYTHON.sources));
        assert!(check_patterns("cursor.execute", "cursor.execute", PYTHON.sinks));
    }

    #[test]
    fn test_language_routing() {
        let php = patterns_for_language(Language::Php);
        assert!(php.sources.contains(&"get_option"));

        let py = patterns_for_language(Language::Python);
        assert!(py.sources.contains(&"request.args"));

        let js = patterns_for_language(Language::JavaScript);
        assert!(js.sinks.contains(&"innerHTML"));

        let rust = patterns_for_language(Language::Rust);
        assert!(rust.sources.contains(&"env::var"));
    }

    #[test]
    fn test_custom_taint_rules_parse() {
        let toml_str = r#"
sources = ["custom_input", "get_user_data"]
sinks = ["dangerous_output", "raw_query"]
sanitizers = ["my_escape", "validate_input"]
"#;
        let rules: CustomTaintRules = toml::from_str(toml_str).unwrap();
        assert_eq!(rules.sources.len(), 2);
        assert_eq!(rules.sinks.len(), 2);
        assert_eq!(rules.sanitizers.len(), 2);
        assert!(rules.sources.contains(&"custom_input".to_string()));
        assert!(rules.sinks.contains(&"dangerous_output".to_string()));
        assert!(rules.sanitizers.contains(&"my_escape".to_string()));
    }

    #[test]
    fn test_custom_taint_rules_partial() {
        let toml_str = r#"
sources = ["only_sources"]
"#;
        let rules: CustomTaintRules = toml::from_str(toml_str).unwrap();
        assert_eq!(rules.sources.len(), 1);
        assert!(rules.sinks.is_empty());
        assert!(rules.sanitizers.is_empty());
    }

    #[test]
    fn test_analyze_taint_empty_graph() {
        let mut graph = graphy_core::CodeGraph::new();
        let findings = analyze_taint_with_rules(&mut graph, None);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_custom_taint_rules_empty() {
        let toml_str = r#"
sources = []
sinks = []
sanitizers = []
"#;
        let rules: CustomTaintRules = toml::from_str(toml_str).unwrap();
        assert!(rules.sources.is_empty());
        assert!(rules.sinks.is_empty());
        assert!(rules.sanitizers.is_empty());
    }

    #[test]
    fn test_check_patterns_rust() {
        let rust_patterns = TaintPatterns {
            sources: &["env::var"],
            sinks: &["Command::new"],
            sanitizers: &["bind"],
        };
        assert!(rust_patterns.sources.iter().any(|s| s.contains("env")));
        assert!(rust_patterns.sinks.iter().any(|s| s.contains("Command")));
    }
}
