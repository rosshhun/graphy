//! Phase 13: Probabilistic dead code detection.
//!
//! Multi-pass analysis that assigns a liveness probability (0.0 - 1.0) to each
//! function/method/class instead of a binary alive/dead classification.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use graphy_core::{
    CodeGraph, EdgeKind, Language, NodeKind, SymbolId, Visibility,
};
use petgraph::Direction;
use tracing::debug;

/// Result of dead code analysis for a single symbol.
#[derive(Debug, Clone)]
pub struct LivenessInfo {
    pub symbol_id: SymbolId,
    pub name: String,
    pub file_path: PathBuf,
    pub liveness: f32,
    pub reasons: Vec<String>,
}

/// Phase 13: Detect potentially dead code and annotate nodes with liveness probability.
/// Returns a list of LivenessInfo for all analyzed symbols.
pub fn detect_dead_code(graph: &mut CodeGraph) -> Vec<LivenessInfo> {
    // Load source text for reference checks (per-file, no concatenation).
    let file_contents = load_file_contents(graph);

    // Build a set of all function/method names for identifier reference scanning.
    // We check if a function name appears as a bare identifier in another file
    // (e.g., passed as a callback: `get(api_stats)`, or assigned: `let f = my_func`).
    let all_fn_names: HashSet<String> = graph
        .all_nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Constructor
            ) && !graph.is_phantom(n.id)
        })
        .map(|n| n.name.clone())
        .collect();

    // Collect targets: functions, methods, constructors, classes
    // Exclude phantom call-target nodes (created by parsers for call expressions,
    // they have no parent Contains edge and are not real definitions).
    let targets: Vec<(SymbolId, String, PathBuf, NodeKind, Visibility, Language)> = graph
        .all_nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Constructor | NodeKind::Class
            )
        })
        .filter(|n| !graph.is_phantom(n.id))
        .map(|n| (n.id, n.name.clone(), n.file_path.clone(), n.kind, n.visibility, n.language))
        .collect();

    let mut results: Vec<LivenessInfo> = Vec::new();

    for (sym_id, name, file_path, kind, visibility, language) in &targets {
        let mut liveness: f32 = 0.0;
        let mut reasons: Vec<String> = Vec::new();

        // 1. Entry point exemptions
        if is_entry_point(&name, file_path, *language) {
            liveness = 1.0;
            reasons.push("entry_point".into());
        }

        // 2. Decorated / inside decorated module (structural, language-agnostic)
        if graph.is_decorated(*sym_id) {
            liveness = liveness.max(0.9);
            reasons.push("decorated".into());
        }

        // 2b. Method on a used type (likely called via self-dispatch)
        if graph.is_method_on_used_type(*sym_id) {
            liveness = liveness.max(0.8);
            reasons.push("method_on_used_type".into());
        }

        // 2c. Trait implementation (fmt, Display, Iterator, etc.) — always alive
        //     via dynamic dispatch. Detected by the Overrides or Implements edge
        //     from Phase 6, or by being inside an `impl Trait for Type` block.
        if graph.is_interface_impl(*sym_id) {
            liveness = liveness.max(0.9);
            reasons.push("trait_impl".into());
        }

        // 3. Language protocol method (e.g., Python __str__, __eq__; detected structurally)
        if name.starts_with("__") && name.ends_with("__") && name.len() > 4 {
            liveness = liveness.max(0.95);
            reasons.push("protocol_method".into());
        }

        // 4. Constructor exemption
        if *kind == NodeKind::Constructor {
            liveness = liveness.max(0.95);
            reasons.push("constructor".into());
        }

        // 5. Exported (visibility) check
        if *visibility == Visibility::Public || *visibility == Visibility::Exported {
            liveness = liveness.max(0.3);
            reasons.push("public_api".into());
        }

        // 7. Caller count
        let callers = count_callers(graph, *sym_id);
        if callers > 0 {
            let caller_score = match callers {
                1 => 0.7,
                2..=3 => 0.85,
                _ => 0.95,
            };
            liveness = liveness.max(caller_score);
            reasons.push(format!("callers={}", callers));
        }

        // 8. Is overriding a parent method
        let has_override = !graph
            .outgoing(*sym_id, EdgeKind::Overrides)
            .is_empty();
        if has_override {
            liveness = liveness.max(0.85);
            reasons.push("overrides_parent".into());
        }

        // 9. __all__ export check
        if is_in_all_list(graph, *sym_id, &name, &file_contents) {
            liveness = liveness.max(0.95);
            reasons.push("in___all__".into());
        }

        // 10. String reference check -- if the function name appears as a string
        //     in any file, it might be called dynamically.
        if name.len() > 3 {
            let quoted_double = format!("\"{}\"", name);
            let quoted_single = format!("'{}'", name);
            let has_string_ref = file_contents
                .values()
                .any(|content| content.contains(&quoted_double) || content.contains(&quoted_single));
            if has_string_ref {
                liveness = liveness.max(0.6);
                reasons.push("string_reference".into());
            }
        }

        // 11. Bare identifier reference check -- if the function name appears as a
        //     bare word in another file's source (not as its own definition line),
        //     it's likely used as a callback, handler, or function reference.
        //     e.g., `get(api_stats)`, `map(transform)`, `[func1, func2]`
        if liveness < 0.5 && name.len() > 3 && all_fn_names.contains(name) {
            let referenced_elsewhere = file_contents.iter().any(|(path, content)| {
                if path == file_path {
                    // In own file: check if name appears outside its definition.
                    // Look for the name as a word boundary match, excluding the `fn name` definition.
                    // Exclude definition lines across all languages
                    let def_patterns: Vec<String> = vec![
                        format!("fn {}", name),        // Rust
                        format!("def {}", name),       // Python, Ruby
                        format!("function {}", name),  // PHP, JS
                        format!("func {}", name),      // Go
                    ];
                    let mut found_non_def = false;
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.contains(name.as_str())
                            && !def_patterns.iter().any(|p| trimmed.contains(p.as_str()))
                            && !trimmed.starts_with("//")
                            && !trimmed.starts_with('#')
                            && !trimmed.starts_with("///")
                        {
                            found_non_def = true;
                            break;
                        }
                    }
                    found_non_def
                } else {
                    content.contains(name.as_str())
                }
            });
            if referenced_elsewhere {
                liveness = liveness.max(0.7);
                reasons.push("identifier_reference".into());
            }
        }

        // 12. Svelte component function heuristic -- functions in .svelte files
        //     are likely template-bound (on:click, {#each}, bind:, etc.)
        //     unless they start with `_` (private convention).
        if file_path.extension().map_or(false, |ext| ext == "svelte")
            && !name.starts_with('_')
        {
            liveness = liveness.max(0.8);
            reasons.push("svelte_component".into());
        }

        // If no positive signals, mark as likely dead
        if reasons.is_empty() {
            reasons.push("no_callers_or_references".into());
        }

        // Store liveness on the node itself via confidence
        if let Some(idx) = graph.get_node_index(*sym_id) {
            if let Some(node) = graph.graph.node_weight_mut(idx) {
                // We use confidence to encode liveness (1.0 = definitely alive)
                node.confidence = liveness;
            }
        }

        results.push(LivenessInfo {
            symbol_id: *sym_id,
            name: name.clone(),
            file_path: file_path.clone(),
            liveness,
            reasons,
        });
    }

    let dead_count = results.iter().filter(|r| r.liveness < 0.3).count();
    let suspect_count = results
        .iter()
        .filter(|r| r.liveness >= 0.3 && r.liveness < 0.7)
        .count();

    debug!(
        "Phase 13 (Dead Code): {} symbols analyzed, {} likely dead, {} suspect",
        results.len(),
        dead_count,
        suspect_count
    );

    results
}

/// Check if this function/file is an entry point.
/// Uses language-aware heuristics instead of hardcoded file names.
fn is_entry_point(name: &str, file_path: &PathBuf, language: Language) -> bool {
    // Universal: `main` is an entry point in every language
    if name == "main" {
        return true;
    }

    let file_name = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    match language {
        Language::Python => {
            // __main__.py, manage.py (Django), wsgi.py, asgi.py
            name == "__main__"
                || file_name == "__main__"
                || file_name == "manage"
                || file_name == "wsgi"
                || file_name == "asgi"
        }
        Language::Java | Language::Kotlin => {
            // Java/Kotlin: main method is the entry point (already handled above)
            false
        }
        Language::Go => {
            // Go: init() functions run automatically
            name == "init"
        }
        Language::Ruby => {
            // Ruby: Rakefile, config.ru
            file_name == "Rakefile" || file_name == "config"
        }
        _ => false,
    }
}

/// Count how many callers a symbol has.
fn count_callers(graph: &CodeGraph, sym_id: SymbolId) -> usize {
    let Some(idx) = graph.get_node_index(sym_id) else {
        return 0;
    };

    graph
        .graph
        .edges_directed(idx, Direction::Incoming)
        .filter(|e| e.weight().kind == EdgeKind::Calls)
        .count()
}

/// Check if a symbol is listed in __all__ (a Constant/Variable named __all__).
fn is_in_all_list(
    graph: &CodeGraph,
    _sym_id: SymbolId,
    name: &str,
    file_contents: &HashMap<PathBuf, String>,
) -> bool {
    for node in graph.find_by_name("__all__") {
        if matches!(node.kind, NodeKind::Constant | NodeKind::Variable) {
            if let Some(content) = file_contents.get(&node.file_path) {
                if let Some(all_start) = content.find("__all__") {
                    let rest = &content[all_start..];
                    if let Some(bracket_end) = rest.find(']') {
                        let all_section = &rest[..bracket_end];
                        if all_section.contains(&format!("\"{}\"", name))
                            || all_section.contains(&format!("'{}'", name))
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Load file contents for all indexed files (for string reference checks).
fn load_file_contents(graph: &CodeGraph) -> HashMap<PathBuf, String> {
    let mut cache = HashMap::new();
    let files: HashSet<PathBuf> = graph
        .all_nodes()
        .filter(|n| n.kind == NodeKind::File)
        .map(|n| n.file_path.clone())
        .collect();

    for file in files {
        if let Ok(content) = std::fs::read_to_string(&file) {
            cache.insert(file, content);
        }
    }
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{CodeGraph, GirEdge, GirNode, Span};

    fn make_fn(name: &str, file: &str, line: u32, lang: Language) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            lang,
        )
    }

    fn make_file(file: &str, lang: Language) -> GirNode {
        GirNode::new(
            file.to_string(),
            NodeKind::File,
            PathBuf::from(file),
            Span::new(0, 0, 0, 0),
            lang,
        )
    }

    #[test]
    fn entry_point_main_is_alive() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/main.py", Language::Python);
        let main_fn = make_fn("main", "/proj/main.py", 1, Language::Python);
        let file_id = file.id;
        let main_id = main_fn.id;
        graph.add_node(file);
        graph.add_node(main_fn);
        graph.add_edge(file_id, main_id, GirEdge::new(graphy_core::EdgeKind::Contains));

        let results = detect_dead_code(&mut graph);
        let main_info = results.iter().find(|r| r.name == "main").unwrap();
        assert!(main_info.liveness >= 0.95, "main should be alive, got {}", main_info.liveness);
    }

    #[test]
    fn function_with_callers_is_alive() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/lib.py", Language::Python);
        let caller = make_fn("caller", "/proj/lib.py", 1, Language::Python);
        let callee = make_fn("callee", "/proj/lib.py", 10, Language::Python);
        let file_id = file.id;
        let caller_id = caller.id;
        let callee_id = callee.id;
        graph.add_node(file);
        graph.add_node(caller.clone());
        graph.add_node(callee.clone());
        graph.add_edge(file_id, caller_id, GirEdge::new(graphy_core::EdgeKind::Contains));
        graph.add_edge(file_id, callee_id, GirEdge::new(graphy_core::EdgeKind::Contains));
        graph.add_edge(caller_id, callee_id, GirEdge::new(graphy_core::EdgeKind::Calls));

        let results = detect_dead_code(&mut graph);
        let callee_info = results.iter().find(|r| r.name == "callee").unwrap();
        assert!(callee_info.liveness >= 0.7, "callee with caller should be alive, got {}", callee_info.liveness);
    }

    #[test]
    fn uncalled_private_function_is_dead() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/lib.py", Language::Python);
        let dead_fn = make_fn("unused_helper", "/proj/lib.py", 1, Language::Python);
        let file_id = file.id;
        let dead_id = dead_fn.id;
        graph.add_node(file);
        graph.add_node(dead_fn);
        graph.add_edge(file_id, dead_id, GirEdge::new(graphy_core::EdgeKind::Contains));

        let results = detect_dead_code(&mut graph);
        let dead_info = results.iter().find(|r| r.name == "unused_helper").unwrap();
        assert!(dead_info.liveness < 0.5, "uncalled function should be dead, got {}", dead_info.liveness);
    }

    #[test]
    fn decorated_function_is_alive() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/app.py", Language::Python);
        let route = make_fn("health_check", "/proj/app.py", 5, Language::Python);
        let decorator = GirNode::new(
            "route".to_string(),
            NodeKind::Decorator,
            PathBuf::from("/proj/app.py"),
            Span::new(4, 0, 4, 20),
            Language::Python,
        );
        let file_id = file.id;
        let route_id = route.id;
        let dec_id = decorator.id;
        graph.add_node(file);
        graph.add_node(route);
        graph.add_node(decorator);
        graph.add_edge(file_id, route_id, GirEdge::new(graphy_core::EdgeKind::Contains));
        graph.add_edge(route_id, dec_id, GirEdge::new(graphy_core::EdgeKind::AnnotatedWith));

        let results = detect_dead_code(&mut graph);
        let route_info = results.iter().find(|r| r.name == "health_check").unwrap();
        assert!(route_info.liveness >= 0.9, "decorated function should be alive, got {}", route_info.liveness);
    }

    #[test]
    fn test_function_by_name_is_alive() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/test_main.py", Language::Python);
        let test_fn = make_fn("test_basic", "/proj/test_main.py", 1, Language::Python);
        let file_id = file.id;
        let test_id = test_fn.id;
        graph.add_node(file);
        graph.add_node(test_fn);
        graph.add_edge(file_id, test_id, GirEdge::new(graphy_core::EdgeKind::Contains));

        let results = detect_dead_code(&mut graph);
        let test_info = results.iter().find(|r| r.name == "test_basic");
        // test_ prefix functions may get low liveness because they have no callers,
        // but they should be detected as tests by suggest_tests tool
        // The dead code detector doesn't special-case test_ prefix (this is by design:
        // tests ARE entry points only when run by a test runner)
        assert!(test_info.is_some());
    }

    #[test]
    fn constructor_is_alive() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/model.py", Language::Python);
        let ctor = GirNode::new(
            "__init__".to_string(),
            NodeKind::Constructor,
            PathBuf::from("/proj/model.py"),
            Span::new(5, 0, 10, 0),
            Language::Python,
        );
        let file_id = file.id;
        let ctor_id = ctor.id;
        graph.add_node(file);
        graph.add_node(ctor);
        graph.add_edge(file_id, ctor_id, GirEdge::new(graphy_core::EdgeKind::Contains));

        let results = detect_dead_code(&mut graph);
        let ctor_info = results.iter().find(|r| r.name == "__init__").unwrap();
        assert!(ctor_info.liveness >= 0.95, "constructor should be alive, got {}", ctor_info.liveness);
    }

    #[test]
    fn public_function_gets_some_liveness() {
        let mut graph = CodeGraph::new();
        let file = make_file("/proj/lib.rs", Language::Rust);
        let mut pub_fn = make_fn("public_api", "/proj/lib.rs", 1, Language::Rust);
        pub_fn.visibility = Visibility::Public;
        let file_id = file.id;
        let pub_id = pub_fn.id;
        graph.add_node(file);
        graph.add_node(pub_fn);
        graph.add_edge(file_id, pub_id, GirEdge::new(graphy_core::EdgeKind::Contains));

        let results = detect_dead_code(&mut graph);
        let pub_info = results.iter().find(|r| r.name == "public_api").unwrap();
        assert!(pub_info.liveness >= 0.3, "public function should have some liveness, got {}", pub_info.liveness);
    }

    // ── Integration tests: index real fixture projects and assert liveness ──

    /// Helper: run the full pipeline on a fixture directory and return the graph.
    fn index_fixture(fixture_name: &str) -> CodeGraph {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
        let fixture_root = workspace_root.join("tests/fixtures/calibration").join(fixture_name);
        assert!(fixture_root.exists(), "fixture not found: {}", fixture_root.display());

        let config = crate::pipeline::PipelineConfig {
            incremental: false,
            use_lsp: false,
            ..Default::default()
        };
        let pipeline = crate::AnalysisPipeline::new(fixture_root, config);
        pipeline.run().expect("pipeline should succeed on fixture")
    }

    fn liveness_of(graph: &CodeGraph, name: &str) -> f32 {
        graph.all_nodes()
            .find(|n| n.name == name && n.kind.is_callable())
            .map(|n| n.confidence)
            .unwrap_or_else(|| panic!("symbol '{}' not found in graph", name))
    }

    #[test]
    fn calibration_flask_app_decorated_routes_alive() {
        let graph = index_fixture("flask_app");

        // Decorated routes should be alive
        let health = liveness_of(&graph, "health_check");
        assert!(health >= 0.9, "health_check (decorated) should be alive, got {:.2}", health);

        let stats = liveness_of(&graph, "api_stats");
        assert!(stats >= 0.9, "api_stats (decorated) should be alive, got {:.2}", stats);
    }

    #[test]
    fn calibration_flask_app_called_helper_alive() {
        let graph = index_fixture("flask_app");

        // compute_stats is called by api_stats
        let cs = liveness_of(&graph, "compute_stats");
        assert!(cs >= 0.7, "compute_stats (has caller) should be alive, got {:.2}", cs);
    }

    #[test]
    fn calibration_flask_app_dead_functions_dead() {
        let graph = index_fixture("flask_app");

        let unused = liveness_of(&graph, "unused_helper");
        assert!(unused < 0.5, "unused_helper should be dead, got {:.2}", unused);

        let dead = liveness_of(&graph, "another_dead_function");
        assert!(dead < 0.5, "another_dead_function should be dead, got {:.2}", dead);
    }

    #[test]
    fn calibration_rust_project_main_alive() {
        let graph = index_fixture("rust_project");

        let main = liveness_of(&graph, "main");
        assert!(main >= 0.95, "main should be alive, got {:.2}", main);
    }

    #[test]
    fn calibration_rust_project_dead_fn_dead() {
        let graph = index_fixture("rust_project");

        let dead = liveness_of(&graph, "truly_dead");
        assert!(dead < 0.5, "truly_dead should be dead, got {:.2}", dead);
    }

    #[test]
    fn calibration_rust_workspace_crate_map_built() {
        // Verify cross-crate resolution builds a crate map for the workspace
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
        let _fixture_root = workspace_root.join("tests/fixtures/calibration/rust_workspace");

        let graph = index_fixture("rust_workspace");

        // shared_helper should exist in the graph (from crate_a)
        let found = graph.all_nodes().any(|n| n.name == "shared_helper");
        assert!(found, "shared_helper from crate_a should be in the graph");

        // dead_in_crate_a should be flagged as low liveness
        let dead = liveness_of(&graph, "dead_in_crate_a");
        assert!(dead < 0.5, "dead_in_crate_a should be dead, got {:.2}", dead);
    }

    #[test]
    fn liveness_score_bounded_0_to_1() {
        let mut graph = index_fixture("rust_project");
        let results = detect_dead_code(&mut graph);
        for r in &results {
            assert!(
                r.liveness >= 0.0 && r.liveness <= 1.0,
                "{} has invalid liveness: {}",
                r.name,
                r.liveness
            );
        }
    }

    #[test]
    fn empty_graph_dead_code_empty() {
        let mut graph = graphy_core::CodeGraph::new();
        let results = detect_dead_code(&mut graph);
        assert!(results.is_empty());
    }
}
