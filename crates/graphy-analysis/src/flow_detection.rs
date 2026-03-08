//! Phase 12: Process/flow detection.
//!
//! Detect entry points by framework patterns, then BFS through the call graph
//! to trace execution flows and label each flow with its entry point.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use graphy_core::{
    CodeGraph, EdgeKind, Language, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// A detected execution flow rooted at an entry point.
#[derive(Debug, Clone)]
pub struct ExecutionFlow {
    pub entry_point: SymbolId,
    pub entry_name: String,
    pub entry_file: PathBuf,
    pub flow_kind: FlowKind,
    /// All symbol IDs reachable from this entry point via the call graph.
    pub reachable: Vec<SymbolId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowKind {
    WebRoute,
    CliCommand,
    TestCase,
    MainEntry,
    ScriptEntry,
    TaskHandler,
    Generic,
}

/// Framework entry-point patterns (decorator/annotation text fragments).
/// Cross-language: covers Python (Flask, FastAPI, Django), JS/TS (Express),
/// Java (Spring), Ruby (Sinatra), and more.
const ROUTE_PATTERNS: &[&str] = &[
    // Python: Flask, FastAPI, Django
    "app.route", "router.", "api_view",
    "app.get", "app.post", "app.put", "app.delete", "app.patch",
    "blueprint.route", "api.route",
    // Java/Kotlin: Spring
    "GetMapping", "PostMapping", "PutMapping", "DeleteMapping",
    "PatchMapping", "RequestMapping",
    // Ruby: Sinatra
    "get ", "post ", "put ", "delete ",
];

const CLI_PATTERNS: &[&str] = &[
    "click.command", "click.group",
    "app.command", "typer.command",
    // Rust: clap (detected by attribute, not decorator)
    "command",
];

const TASK_PATTERNS: &[&str] = &[
    "celery.task", "shared_task", "task",
    "dramatiq.actor",
    // Java: Spring @Scheduled, @Async
    "Scheduled", "Async",
];

/// Phase 12: Detect entry points and trace execution flows.
pub fn detect_flows(graph: &mut CodeGraph) -> Vec<ExecutionFlow> {
    let mut entry_points: Vec<(SymbolId, String, PathBuf, FlowKind)> = Vec::new();

    // Scan all callable nodes for entry point patterns
    for node in graph.all_nodes() {
        if !matches!(
            node.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Constructor
        ) {
            continue;
        }

        let sym_id = node.id;
        let name = &node.name;
        let file = &node.file_path;
        let lang = node.language;

        // Check 1: test functions (language-aware patterns)
        if is_test_function(name, file, lang) {
            entry_points.push((sym_id, name.clone(), file.clone(), FlowKind::TestCase));
            continue;
        }

        // Check 2: main() function
        if name == "main" {
            entry_points.push((sym_id, name.clone(), file.clone(), FlowKind::MainEntry));
            continue;
        }

        // Check 3: __main__ script entry
        let file_stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_stem == "__main__" {
            entry_points.push((sym_id, name.clone(), file.clone(), FlowKind::ScriptEntry));
            continue;
        }

        // Check 4: decorator-based entry points
        let decorators = get_decorator_names(graph, sym_id);
        let flow_kind = classify_by_decorators(&decorators);
        if let Some(kind) = flow_kind {
            entry_points.push((sym_id, name.clone(), file.clone(), kind));
            continue;
        }

        // Check 5: if __name__ == "__main__" pattern -- look at file-level scope
        // The parser may have captured this as a function call; we detect it
        // via the file name and check for a top-level function.
    }

    // Check for `if __name__ == "__main__"` by scanning File nodes
    for file_node in graph.find_by_kind(NodeKind::File) {
        let file_path = &file_node.file_path;
        if let Ok(content) = std::fs::read_to_string(file_path) {
            if content.contains("if __name__") && content.contains("__main__") {
                // Find top-level functions in this file that are called from the if block
                // As a heuristic, just mark the file's top-level functions as potential entry points
                for child in graph.children(file_node.id) {
                    if child.kind == NodeKind::Function && child.name == "main" {
                        // Already handled above, skip duplicate
                        continue;
                    }
                }
                // Mark the file itself as having a script entry
                // We can find the first function called in the if block
                // but for simplicity, we just note the file has __main__ guard
            }
        }
    }

    // BFS from each entry point through the call graph
    let mut flows: Vec<ExecutionFlow> = Vec::new();

    for (entry_id, entry_name, entry_file, flow_kind) in &entry_points {
        let reachable = bfs_call_graph(graph, *entry_id);

        flows.push(ExecutionFlow {
            entry_point: *entry_id,
            entry_name: entry_name.clone(),
            entry_file: entry_file.clone(),
            flow_kind: flow_kind.clone(),
            reachable,
        });
    }

    debug!(
        "Phase 12 (Flow Detection): {} entry points, {} flows traced",
        entry_points.len(),
        flows.len()
    );

    flows
}

/// Check if a function is a test based on language conventions.
fn is_test_function(name: &str, file: &PathBuf, lang: Language) -> bool {
    match lang {
        Language::Python => {
            // pytest: test_*, unittest: Test* class
            name.starts_with("test_") || name.starts_with("Test")
        }
        Language::Rust => {
            // Rust tests are detected by #[test] attribute (AnnotatedWith edge),
            // but name convention is also common
            name.starts_with("test_")
                || file.to_string_lossy().contains("tests/")
                || file.to_string_lossy().ends_with("_test.rs")
        }
        Language::Go => {
            // Go: Test* functions in *_test.go files
            name.starts_with("Test")
                && file
                    .file_name()
                    .map_or(false, |f| f.to_string_lossy().ends_with("_test.go"))
        }
        Language::Java | Language::Kotlin => {
            // JUnit: @Test annotation (handled by decorator check), but name patterns too
            name.starts_with("test")
                || file.to_string_lossy().contains("test/")
                || file.to_string_lossy().contains("Test")
        }
        Language::Ruby => {
            // Minitest: test_*, RSpec: handled by decorators
            name.starts_with("test_")
        }
        Language::JavaScript | Language::TypeScript | Language::Svelte => {
            // Jest/Vitest: test files detected by convention
            let fname = file.to_string_lossy();
            (name == "it" || name == "test" || name == "describe")
                || fname.contains(".test.")
                || fname.contains(".spec.")
                || fname.contains("__tests__")
        }
        _ => {
            // Fallback: common test_ prefix
            name.starts_with("test_") || name.starts_with("Test")
        }
    }
}

/// BFS through the call graph from a starting node.
fn bfs_call_graph(graph: &CodeGraph, start: SymbolId) -> Vec<SymbolId> {
    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut queue: VecDeque<SymbolId> = VecDeque::new();

    visited.insert(start);
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        let Some(idx) = graph.get_node_index(current) else {
            continue;
        };

        for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
            if edge.weight().kind == EdgeKind::Calls {
                let target_idx = edge.target();
                if let Some(target_node) = graph.graph.node_weight(target_idx) {
                    if visited.insert(target_node.id) {
                        queue.push_back(target_node.id);
                    }
                }
            }
        }
    }

    // Remove the start node from the result (it's the entry point itself)
    visited.remove(&start);
    visited.into_iter().collect()
}

/// Get decorator names for a given symbol.
fn get_decorator_names(graph: &CodeGraph, sym_id: SymbolId) -> Vec<String> {
    graph
        .outgoing(sym_id, EdgeKind::AnnotatedWith)
        .iter()
        .map(|n| n.name.clone())
        .collect()
}

/// Classify a function's role based on its decorators.
fn classify_by_decorators(decorators: &[String]) -> Option<FlowKind> {
    for dec in decorators {
        let dec_lower = dec.to_lowercase();

        for pattern in ROUTE_PATTERNS {
            if dec_lower.contains(&pattern.to_lowercase()) {
                return Some(FlowKind::WebRoute);
            }
        }

        for pattern in CLI_PATTERNS {
            if dec_lower.contains(&pattern.to_lowercase()) {
                return Some(FlowKind::CliCommand);
            }
        }

        for pattern in TASK_PATTERNS {
            if dec_lower.contains(&pattern.to_lowercase()) {
                return Some(FlowKind::TaskHandler);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirEdge, GirNode, Span};

    fn make_fn(name: &str, file: &str, line: u32, lang: Language) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            lang,
        )
    }

    #[test]
    fn detect_flows_empty_graph() {
        let mut graph = CodeGraph::new();
        let flows = detect_flows(&mut graph);
        assert!(flows.is_empty());
    }

    #[test]
    fn detect_flows_main_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("main", "src/main.rs", 1, Language::Rust));
        let flows = detect_flows(&mut graph);
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].flow_kind, FlowKind::MainEntry);
        assert_eq!(flows[0].entry_name, "main");
    }

    #[test]
    fn detect_flows_test_functions() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("test_something", "tests/test_app.py", 1, Language::Python));
        graph.add_node(make_fn("TestClass", "tests/test_app.py", 10, Language::Python));
        let flows = detect_flows(&mut graph);
        assert_eq!(flows.len(), 2);
        assert!(flows.iter().all(|f| f.flow_kind == FlowKind::TestCase));
    }

    #[test]
    fn detect_flows_with_callees() {
        let mut graph = CodeGraph::new();
        let main_fn = make_fn("main", "main.py", 1, Language::Python);
        let helper = make_fn("helper", "main.py", 10, Language::Python);
        let main_id = main_fn.id;
        let helper_id = helper.id;
        graph.add_node(main_fn);
        graph.add_node(helper);
        graph.add_edge(main_id, helper_id, GirEdge::new(EdgeKind::Calls));

        let flows = detect_flows(&mut graph);
        assert_eq!(flows.len(), 1);
        assert!(flows[0].reachable.contains(&helper_id));
    }

    #[test]
    fn is_test_function_python() {
        assert!(is_test_function("test_foo", &PathBuf::from("app.py"), Language::Python));
        assert!(is_test_function("TestCase", &PathBuf::from("app.py"), Language::Python));
        assert!(!is_test_function("helper", &PathBuf::from("app.py"), Language::Python));
    }

    #[test]
    fn is_test_function_rust() {
        assert!(is_test_function("test_something", &PathBuf::from("src/lib.rs"), Language::Rust));
        assert!(is_test_function("func", &PathBuf::from("tests/integration.rs"), Language::Rust));
        assert!(!is_test_function("helper", &PathBuf::from("src/lib.rs"), Language::Rust));
    }

    #[test]
    fn is_test_function_go() {
        assert!(is_test_function("TestFoo", &PathBuf::from("foo_test.go"), Language::Go));
        assert!(!is_test_function("TestFoo", &PathBuf::from("foo.go"), Language::Go));
        assert!(!is_test_function("helper", &PathBuf::from("foo_test.go"), Language::Go));
    }

    #[test]
    fn is_test_function_javascript() {
        assert!(is_test_function("test", &PathBuf::from("app.test.js"), Language::JavaScript));
        assert!(is_test_function("it", &PathBuf::from("app.spec.ts"), Language::TypeScript));
        assert!(!is_test_function("render", &PathBuf::from("app.js"), Language::JavaScript));
    }

    #[test]
    fn classify_by_decorators_route() {
        let decs = vec!["app.route(\"/api\")".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::WebRoute));
    }

    #[test]
    fn classify_by_decorators_cli() {
        let decs = vec!["click.command()".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::CliCommand));
    }

    #[test]
    fn classify_by_decorators_task() {
        let decs = vec!["celery.task".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::TaskHandler));
    }

    #[test]
    fn classify_by_decorators_none() {
        let decs = vec!["staticmethod".to_string()];
        assert_eq!(classify_by_decorators(&decs), None);
    }

    #[test]
    fn classify_by_decorators_empty() {
        assert_eq!(classify_by_decorators(&[]), None);
    }

    #[test]
    fn bfs_call_graph_no_edges() {
        let mut graph = CodeGraph::new();
        let f = make_fn("isolated", "a.py", 1, Language::Python);
        let f_id = f.id;
        graph.add_node(f);
        let reachable = bfs_call_graph(&graph, f_id);
        assert!(reachable.is_empty());
    }

    #[test]
    fn bfs_call_graph_chain() {
        let mut graph = CodeGraph::new();
        let a = make_fn("a", "a.py", 1, Language::Python);
        let b = make_fn("b", "a.py", 10, Language::Python);
        let c = make_fn("c", "a.py", 20, Language::Python);
        let a_id = a.id;
        let b_id = b.id;
        let c_id = c.id;
        graph.add_node(a);
        graph.add_node(b);
        graph.add_node(c);
        graph.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Calls));
        graph.add_edge(b_id, c_id, GirEdge::new(EdgeKind::Calls));

        let reachable = bfs_call_graph(&graph, a_id);
        assert_eq!(reachable.len(), 2);
        assert!(reachable.contains(&b_id));
        assert!(reachable.contains(&c_id));
    }

    #[test]
    fn bfs_call_graph_cycle() {
        let mut graph = CodeGraph::new();
        let a = make_fn("a", "a.py", 1, Language::Python);
        let b = make_fn("b", "a.py", 10, Language::Python);
        let a_id = a.id;
        let b_id = b.id;
        graph.add_node(a);
        graph.add_node(b);
        graph.add_edge(a_id, b_id, GirEdge::new(EdgeKind::Calls));
        graph.add_edge(b_id, a_id, GirEdge::new(EdgeKind::Calls));

        // Should not infinite loop
        let reachable = bfs_call_graph(&graph, a_id);
        assert_eq!(reachable.len(), 1);
        assert!(reachable.contains(&b_id));
    }

    #[test]
    fn flow_kind_equality() {
        assert_eq!(FlowKind::WebRoute, FlowKind::WebRoute);
        assert_ne!(FlowKind::WebRoute, FlowKind::MainEntry);
    }

    #[test]
    fn detect_flows_flask_route() {
        let mut graph = CodeGraph::new();
        let handler = make_fn("index", "app.py", 1, Language::Python);
        let handler_id = handler.id;
        graph.add_node(handler);

        // Add decorator node matching a route pattern (lowercase match)
        let dec = GirNode::new(
            "app.route(\"/\")".to_string(),
            NodeKind::Decorator,
            PathBuf::from("app.py"),
            Span::new(0, 0, 0, 30),
            Language::Python,
        );
        let dec_id = dec.id;
        graph.add_node(dec);
        graph.add_edge(handler_id, dec_id, GirEdge::new(EdgeKind::AnnotatedWith));

        let flows = detect_flows(&mut graph);
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].flow_kind, FlowKind::WebRoute);
    }

    #[test]
    fn classify_spring_annotations() {
        // Spring annotations like GetMapping should match WebRoute
        let decs = vec!["GetMapping".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::WebRoute));

        let decs = vec!["PostMapping(\"/api/users\")".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::WebRoute));

        let decs = vec!["RequestMapping".to_string()];
        assert_eq!(classify_by_decorators(&decs), Some(FlowKind::WebRoute));
    }
}
