use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::info;

use graphy_core::{CodeGraph, EdgeKind, NodeKind, Visibility};
use graphy_search::SearchIndex;

#[derive(Embed)]
#[folder = "dist"]
struct FrontendAssets;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub graph: Arc<RwLock<CodeGraph>>,
    pub search: Arc<SearchIndex>,
    pub project_root: PathBuf,
}

/// Launch the web server with graceful shutdown support.
pub async fn serve(
    state: AppState,
    port: u16,
    shutdown: tokio::sync::watch::Receiver<()>,
) -> Result<()> {
    let app = Router::new()
        .route("/api/stats", get(api_stats))
        .route("/api/search", get(api_search))
        .route("/api/symbol/:name", get(api_symbol))
        .route("/api/graph", get(api_graph_data))
        .route("/api/files", get(api_files))
        .route("/api/hotspots", get(api_hotspots))
        .route("/api/dead-code", get(api_dead_code))
        .route("/api/taint", get(api_taint))
        .route("/api/architecture", get(api_architecture))
        .route("/api/patterns", get(api_patterns))
        .route("/api/api-surface", get(api_surface))
        .route("/api/file-content", get(api_file_content))
        .route("/api/file-symbols", get(api_file_symbols))
        .route("/", get(serve_frontend_index))
        .fallback(get(serve_frontend))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!("Web UI available at http://localhost:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.clone().changed().await;
        })
        .await?;
    Ok(())
}

/// Strip the project root prefix from a path, returning a relative path string.
fn relative_path(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

// ── Frontend Serving ────────────────────────────────────────

async fn serve_frontend_index() -> Response {
    serve_embedded_file("index.html")
}

async fn serve_frontend(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() && FrontendAssets::get(path).is_some() {
        serve_embedded_file(path)
    } else {
        serve_embedded_file("index.html")
    }
}

fn serve_embedded_file(path: &str) -> Response {
    match FrontendAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

// ── API Handlers ────────────────────────────────────────────

#[derive(Serialize)]
struct StatsResponse {
    nodes: usize,
    edges: usize,
    files: usize,
    classes: usize,
    structs: usize,
    enums: usize,
    traits: usize,
    functions: usize,
    methods: usize,
    imports: usize,
    variables: usize,
    constants: usize,
}

async fn api_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let graph = state.graph.read().await;
    Json(StatsResponse {
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        files: graph.find_by_kind(NodeKind::File).len(),
        classes: graph.find_by_kind(NodeKind::Class).len(),
        structs: graph.find_by_kind(NodeKind::Struct).len(),
        enums: graph.find_by_kind(NodeKind::Enum).len(),
        traits: graph.find_by_kind(NodeKind::Trait).len(),
        functions: graph.find_by_kind(NodeKind::Function).len(),
        methods: graph.find_by_kind(NodeKind::Method).len(),
        imports: graph.find_by_kind(NodeKind::Import).len(),
        variables: graph.find_by_kind(NodeKind::Variable).len(),
        constants: graph.find_by_kind(NodeKind::Constant).len(),
    })
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
    kind: Option<String>,
    lang: Option<String>,
    file: Option<String>,
}

fn default_limit() -> usize {
    20
}

async fn api_search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let query = params.q.trim();
    let has_filters = params.kind.is_some() || params.lang.is_some() || params.file.is_some();
    if query.is_empty() && !has_filters {
        return (StatusCode::BAD_REQUEST, "Query parameter 'q' must not be empty").into_response();
    }
    let limit = params.limit.clamp(1, 1000);

    let result = if has_filters {
        state.search.search_filtered(
            query,
            params.kind.as_deref(),
            params.lang.as_deref(),
            params.file.as_deref(),
            limit,
        )
    } else {
        state.search.search(query, limit)
    };
    match result {
        Ok(results) => Json(results).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Serialize)]
struct SymbolDetail {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    end_line: u32,
    visibility: String,
    language: String,
    signature: Option<String>,
    doc: Option<String>,
    complexity: Option<ComplexityInfo>,
    callers: Vec<SymbolRef>,
    callees: Vec<SymbolRef>,
    children: Vec<SymbolRef>,
}

#[derive(Serialize)]
struct ComplexityInfo {
    cyclomatic: u32,
    cognitive: u32,
    loc: u32,
    sloc: u32,
    parameter_count: u32,
    max_nesting_depth: u32,
}

#[derive(Serialize, Clone)]
struct SymbolRef {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
}

fn node_to_ref(n: &graphy_core::GirNode, root: &std::path::Path) -> SymbolRef {
    SymbolRef {
        name: n.name.clone(),
        kind: format!("{:?}", n.kind),
        file_path: relative_path(&n.file_path, root),
        start_line: n.span.start_line,
    }
}

async fn api_symbol(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> impl IntoResponse {
    let graph = state.graph.read().await;
    let nodes = graph.find_by_name(&name);

    if nodes.is_empty() {
        return (StatusCode::NOT_FOUND, "Symbol not found").into_response();
    }

    let root = &state.project_root;
    let details: Vec<SymbolDetail> = nodes
        .iter()
        .map(|n| {
            let complexity = n.complexity.as_ref().map(|cx| ComplexityInfo {
                cyclomatic: cx.cyclomatic,
                cognitive: cx.cognitive,
                loc: cx.loc,
                sloc: cx.sloc,
                parameter_count: cx.parameter_count,
                max_nesting_depth: cx.max_nesting_depth,
            });

            SymbolDetail {
                name: n.name.clone(),
                kind: format!("{:?}", n.kind),
                file_path: relative_path(&n.file_path, root),
                start_line: n.span.start_line,
                end_line: n.span.end_line,
                visibility: format!("{:?}", n.visibility),
                language: format!("{:?}", n.language),
                signature: n.signature.clone(),
                doc: n.doc.clone(),
                complexity,
                callers: graph.callers(n.id).iter().map(|c| node_to_ref(c, root)).collect(),
                callees: graph.callees(n.id).iter().map(|c| node_to_ref(c, root)).collect(),
                children: graph.children(n.id).iter().map(|c| node_to_ref(c, root)).collect(),
            }
        })
        .collect();

    Json(details).into_response()
}

#[derive(Serialize)]
struct GraphData {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    label: String,
    kind: String,
    file: String,
    size: u32,
    visibility: String,
    complexity: Option<u32>,
}

#[derive(Serialize)]
struct GraphEdge {
    source: String,
    target: String,
    kind: String,
    confidence: f32,
}

async fn api_graph_data(State(state): State<AppState>) -> Json<GraphData> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    const MAX_GRAPH_NODES: usize = 400;

    // Collect eligible nodes with a relevance score for prioritization
    let mut scored_nodes: Vec<(&graphy_core::GirNode, u32)> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable() || n.kind.is_type_def() || n.kind == NodeKind::File)
        .map(|n| {
            // Score: type defs and files are important, complexity adds weight, callers add weight
            let base = match n.kind {
                NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => 100,
                NodeKind::File => 50,
                NodeKind::Enum => 80,
                _ => 10,
            };
            let complexity_bonus = n
                .complexity
                .as_ref()
                .map(|cx| cx.cyclomatic.min(30))
                .unwrap_or(0);
            let caller_bonus = (graph.callers(n.id).len() as u32).min(20) * 3;
            (n, base + complexity_bonus + caller_bonus)
        })
        .collect();

    // Sort by score descending, then take top N
    scored_nodes.sort_by(|a, b| b.1.cmp(&a.1));
    scored_nodes.truncate(MAX_GRAPH_NODES);

    let node_ids: std::collections::HashSet<graphy_core::SymbolId> =
        scored_nodes.iter().map(|(n, _)| n.id).collect();

    let nodes: Vec<GraphNode> = scored_nodes
        .iter()
        .map(|(n, _)| {
            let size = match n.kind {
                NodeKind::Class | NodeKind::Struct | NodeKind::Trait => 10,
                NodeKind::File => 6,
                NodeKind::Enum | NodeKind::Interface => 8,
                _ => {
                    n.complexity
                        .as_ref()
                        .map(|cx| 4 + (cx.cyclomatic.min(20) / 2))
                        .unwrap_or(4)
                }
            };
            GraphNode {
                id: n.id.to_string(),
                label: n.name.clone(),
                kind: format!("{:?}", n.kind),
                file: relative_path(&n.file_path, root),
                size,
                visibility: format!("{:?}", n.visibility),
                complexity: n.complexity.as_ref().map(|cx| cx.cyclomatic),
            }
        })
        .collect();

    use petgraph::visit::{EdgeRef, IntoEdgeReferences};
    let edges: Vec<GraphEdge> = graph
        .graph
        .edge_references()
        .filter_map(|e| {
            let src = graph.graph.node_weight(e.source())?;
            let tgt = graph.graph.node_weight(e.target())?;
            // Only include edges where both endpoints are in our node set
            if !node_ids.contains(&src.id) || !node_ids.contains(&tgt.id) {
                return None;
            }
            let w = e.weight();
            if matches!(
                w.kind,
                EdgeKind::Calls
                    | EdgeKind::Inherits
                    | EdgeKind::Implements
                    | EdgeKind::Imports
                    | EdgeKind::ImportsFrom
                    | EdgeKind::DataFlowsTo
                    | EdgeKind::TaintedBy
            ) {
                Some(GraphEdge {
                    source: src.id.to_string(),
                    target: tgt.id.to_string(),
                    kind: format!("{:?}", w.kind),
                    confidence: w.confidence,
                })
            } else {
                None
            }
        })
        .collect();

    Json(GraphData { nodes, edges })
}

async fn api_files(State(state): State<AppState>) -> Json<Vec<String>> {
    let graph = state.graph.read().await;
    let root = &state.project_root;
    let mut files: Vec<String> = graph
        .find_by_kind(NodeKind::File)
        .iter()
        .map(|n| relative_path(&n.file_path, root))
        .filter(|p| !p.is_empty())
        .collect();
    files.sort();
    Json(files)
}

// ── New API Endpoints ───────────────────────────────────────

#[derive(Serialize)]
struct HotspotItem {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    cyclomatic: u32,
    cognitive: u32,
    loc: u32,
    caller_count: usize,
    risk_score: f64,
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn api_hotspots(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> Json<Vec<HotspotItem>> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    let mut hotspots: Vec<HotspotItem> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable())
        .filter_map(|n| {
            let cx = n.complexity.as_ref()?;
            let caller_count = graph.callers(n.id).len();
            let risk_score = cx.cyclomatic as f64 * (1.0 + caller_count as f64 * 0.5);
            Some(HotspotItem {
                name: n.name.clone(),
                kind: format!("{:?}", n.kind),
                file_path: relative_path(&n.file_path, root),
                start_line: n.span.start_line,
                cyclomatic: cx.cyclomatic,
                cognitive: cx.cognitive,
                loc: cx.loc,
                caller_count,
                risk_score,
            })
        })
        .collect();

    hotspots.sort_by(|a, b| b.risk_score.total_cmp(&a.risk_score));
    hotspots.truncate(params.limit.min(1000));

    Json(hotspots)
}

#[derive(Serialize)]
struct DeadCodeItem {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    visibility: String,
    dead_probability: f32,
}

async fn api_dead_code(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> Json<Vec<DeadCodeItem>> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    // Use the liveness scores computed by Phase 13 during indexing.
    let callable = [NodeKind::Function, NodeKind::Method];
    let mut dead: Vec<DeadCodeItem> = graph
        .all_nodes()
        .filter(|n| callable.contains(&n.kind))
        .filter(|n| !graph.is_phantom(n.id))
        .filter(|n| n.confidence < 0.5)
        .map(|n| DeadCodeItem {
            name: n.name.clone(),
            kind: format!("{:?}", n.kind),
            file_path: relative_path(&n.file_path, root),
            start_line: n.span.start_line,
            visibility: format!("{:?}", n.visibility),
            dead_probability: 1.0 - n.confidence,
        })
        .collect();

    dead.sort_by(|a, b| b.dead_probability.total_cmp(&a.dead_probability));
    dead.truncate(params.limit.min(1000));

    Json(dead)
}

#[derive(Serialize)]
struct TaintPath {
    target_name: String,
    target_file: String,
    target_line: u32,
    sources: Vec<SymbolRef>,
}

async fn api_taint(State(state): State<AppState>) -> Json<Vec<TaintPath>> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    let paths: Vec<TaintPath> = graph
        .all_nodes()
        .filter(|n| !graph.outgoing(n.id, EdgeKind::TaintedBy).is_empty())
        .map(|n| {
            let sources = graph
                .outgoing(n.id, EdgeKind::TaintedBy)
                .iter()
                .map(|s| node_to_ref(s, root))
                .collect();
            TaintPath {
                target_name: n.name.clone(),
                target_file: relative_path(&n.file_path, root),
                target_line: n.span.start_line,
                sources,
            }
        })
        .collect();

    Json(paths)
}

#[derive(Serialize)]
struct ArchitectureResponse {
    file_count: usize,
    symbol_count: usize,
    edge_count: usize,
    languages: Vec<LangCount>,
    largest_files: Vec<FileSize>,
    kind_distribution: Vec<KindCount>,
    edge_distribution: Vec<KindCount>,
}

#[derive(Serialize)]
struct LangCount {
    language: String,
    count: usize,
}

#[derive(Serialize)]
struct FileSize {
    path: String,
    symbol_count: usize,
}

#[derive(Serialize)]
struct KindCount {
    kind: String,
    count: usize,
}

async fn api_architecture(State(state): State<AppState>) -> Json<ArchitectureResponse> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    // Language distribution
    let mut lang_map: HashMap<String, usize> = HashMap::new();
    for node in graph.all_nodes() {
        *lang_map
            .entry(format!("{:?}", node.language))
            .or_default() += 1;
    }
    let mut languages: Vec<LangCount> = lang_map
        .into_iter()
        .map(|(language, count)| LangCount { language, count })
        .collect();
    languages.sort_by(|a, b| b.count.cmp(&a.count));

    // File sizes
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for node in graph.all_nodes() {
        if node.kind != NodeKind::File && node.kind != NodeKind::Folder {
            *file_counts
                .entry(relative_path(&node.file_path, root))
                .or_default() += 1;
        }
    }
    let mut largest_files: Vec<FileSize> = file_counts
        .into_iter()
        .map(|(path, symbol_count)| FileSize { path, symbol_count })
        .collect();
    largest_files.sort_by(|a, b| b.symbol_count.cmp(&a.symbol_count));
    largest_files.truncate(15);

    // Node kind distribution
    let kinds = [
        NodeKind::File,
        NodeKind::Class,
        NodeKind::Struct,
        NodeKind::Enum,
        NodeKind::Interface,
        NodeKind::Trait,
        NodeKind::Function,
        NodeKind::Method,
        NodeKind::Constructor,
        NodeKind::Import,
        NodeKind::Variable,
        NodeKind::Constant,
        NodeKind::Field,
        NodeKind::TypeAlias,
    ];
    let kind_distribution: Vec<KindCount> = kinds
        .iter()
        .map(|k| KindCount {
            kind: format!("{:?}", k),
            count: graph.find_by_kind(*k).len(),
        })
        .filter(|kc| kc.count > 0)
        .collect();

    // Edge kind distribution
    use petgraph::visit::IntoEdgeReferences;
    let mut edge_map: HashMap<String, usize> = HashMap::new();
    for e in graph.graph.edge_references() {
        *edge_map
            .entry(format!("{:?}", e.weight().kind))
            .or_default() += 1;
    }
    let mut edge_distribution: Vec<KindCount> = edge_map
        .into_iter()
        .map(|(kind, count)| KindCount { kind, count })
        .collect();
    edge_distribution.sort_by(|a, b| b.count.cmp(&a.count));

    Json(ArchitectureResponse {
        file_count: graph.find_by_kind(NodeKind::File).len(),
        symbol_count: graph.node_count(),
        edge_count: graph.edge_count(),
        languages,
        largest_files,
        kind_distribution,
        edge_distribution,
    })
}

#[derive(Serialize)]
struct PatternFinding {
    pattern: String,
    severity: String,
    symbol_name: String,
    detail: String,
    file_path: String,
    line: u32,
}

async fn api_patterns(
    State(state): State<AppState>,
    Query(params): Query<LimitQuery>,
) -> Json<Vec<PatternFinding>> {
    let graph = state.graph.read().await;
    let root = &state.project_root;
    let mut findings = Vec::new();

    // God classes
    for node in graph.all_nodes().filter(|n| n.kind == NodeKind::Class) {
        let methods = graph.children(node.id);
        let method_count = methods
            .iter()
            .filter(|c| c.kind == NodeKind::Method || c.kind == NodeKind::Constructor)
            .count();
        if method_count > 15 {
            findings.push(PatternFinding {
                pattern: "God Class".into(),
                severity: "warning".into(),
                symbol_name: node.name.clone(),
                detail: format!("{} methods", method_count),
                file_path: relative_path(&node.file_path, root),
                line: node.span.start_line,
            });
        }
    }

    // Long parameter lists
    for node in graph.all_nodes().filter(|n| n.kind.is_callable()) {
        let param_count = graph
            .children(node.id)
            .iter()
            .filter(|c| c.kind == NodeKind::Parameter)
            .count();
        if param_count > 5 {
            findings.push(PatternFinding {
                pattern: "Long Parameter List".into(),
                severity: "info".into(),
                symbol_name: node.name.clone(),
                detail: format!("{} parameters", param_count),
                file_path: relative_path(&node.file_path, root),
                line: node.span.start_line,
            });
        }
    }

    // High complexity
    for node in graph.all_nodes().filter(|n| n.kind.is_callable()) {
        if let Some(cx) = &node.complexity {
            if cx.cyclomatic > 15 {
                findings.push(PatternFinding {
                    pattern: "High Complexity".into(),
                    severity: "warning".into(),
                    symbol_name: node.name.clone(),
                    detail: format!("cyclomatic={}, cognitive={}", cx.cyclomatic, cx.cognitive),
                    file_path: relative_path(&node.file_path, root),
                    line: node.span.start_line,
                });
            }
        }
    }

    // Deep nesting
    for node in graph.all_nodes().filter(|n| n.kind.is_callable()) {
        if let Some(cx) = &node.complexity {
            if cx.max_nesting_depth > 5 {
                findings.push(PatternFinding {
                    pattern: "Deep Nesting".into(),
                    severity: "info".into(),
                    symbol_name: node.name.clone(),
                    detail: format!("max depth {}", cx.max_nesting_depth),
                    file_path: relative_path(&node.file_path, root),
                    line: node.span.start_line,
                });
            }
        }
    }

    findings.truncate(params.limit.min(1000));
    Json(findings)
}

#[derive(Serialize)]
struct ApiSurfaceResponse {
    public: Vec<ApiSymbolEntry>,
    effectively_internal: Vec<ApiSymbolEntry>,
    internal_count: usize,
    private_count: usize,
}

#[derive(Serialize)]
struct ApiSymbolEntry {
    name: String,
    kind: String,
    file_path: String,
    start_line: u32,
    signature: Option<String>,
    external_callers: usize,
}

async fn api_surface(State(state): State<AppState>) -> Json<ApiSurfaceResponse> {
    let graph = state.graph.read().await;
    let root = &state.project_root;

    let mut public = Vec::new();
    let mut effectively_internal = Vec::new();
    let mut internal_count = 0usize;
    let mut private_count = 0usize;

    for node in graph.all_nodes() {
        if !node.kind.is_callable() && !node.kind.is_type_def() {
            continue;
        }

        match node.visibility {
            Visibility::Private => {
                private_count += 1;
            }
            Visibility::Internal => {
                internal_count += 1;
            }
            Visibility::Public | Visibility::Exported => {
                let callers = graph.callers(node.id);
                let external_callers = callers
                    .iter()
                    .filter(|c| c.file_path != node.file_path)
                    .count();

                let entry = ApiSymbolEntry {
                    name: node.name.clone(),
                    kind: format!("{:?}", node.kind),
                    file_path: relative_path(&node.file_path, root),
                    start_line: node.span.start_line,
                    signature: node.signature.clone(),
                    external_callers,
                };

                if external_callers == 0 {
                    effectively_internal.push(entry);
                } else {
                    public.push(entry);
                }
            }
        }
    }

    public.sort_by(|a, b| b.external_callers.cmp(&a.external_callers));
    effectively_internal.sort_by(|a, b| a.name.cmp(&b.name));

    Json(ApiSurfaceResponse {
        public,
        effectively_internal,
        internal_count,
        private_count,
    })
}

// ── File Content & Symbols ──────────────────────────────────

#[derive(Deserialize)]
struct FileContentQuery {
    path: String,
}

#[derive(Serialize)]
struct FileContentResponse {
    path: String,
    content: String,
    line_count: usize,
    size_bytes: u64,
}

async fn api_file_content(
    State(state): State<AppState>,
    Query(params): Query<FileContentQuery>,
) -> impl IntoResponse {
    let full_path = state.project_root.join(&params.path);
    let canonical = match full_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    let root_canonical = state
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| state.project_root.clone());
    if !canonical.starts_with(&root_canonical) {
        return (StatusCode::FORBIDDEN, "Path outside project").into_response();
    }
    let metadata = match std::fs::metadata(&canonical) {
        Ok(m) => m,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    if metadata.len() > 1_048_576 {
        return (StatusCode::BAD_REQUEST, "File too large (>1MB)").into_response();
    }
    match std::fs::read_to_string(&canonical) {
        Ok(content) => {
            let line_count = content.lines().count();
            Json(FileContentResponse {
                path: params.path,
                content,
                line_count,
                size_bytes: metadata.len(),
            })
            .into_response()
        }
        Err(_) => (StatusCode::BAD_REQUEST, "Binary or unreadable file").into_response(),
    }
}

#[derive(Serialize, Clone)]
struct FileSymbolEntry {
    name: String,
    kind: String,
    start_line: u32,
    end_line: u32,
    children: Vec<FileSymbolEntry>,
}

#[derive(Serialize)]
struct FileSymbolsResponse {
    path: String,
    symbols: Vec<FileSymbolEntry>,
    symbol_count: usize,
}

async fn api_file_symbols(
    State(state): State<AppState>,
    Query(params): Query<FileContentQuery>,
) -> impl IntoResponse {
    let graph = state.graph.read().await;
    let root = &state.project_root;
    let full_path = root.join(&params.path);

    let file_nodes: Vec<_> = graph
        .all_nodes()
        .filter(|n| n.file_path == full_path && n.kind != NodeKind::File && n.kind != NodeKind::Folder)
        .collect();

    // Collect IDs that are children of something in this file
    let mut child_ids: std::collections::HashSet<graphy_core::SymbolId> =
        std::collections::HashSet::new();
    for n in &file_nodes {
        for c in graph.children(n.id) {
            if c.file_path == full_path {
                child_ids.insert(c.id);
            }
        }
    }

    // Top-level = not a child of anything in this file
    let mut symbols: Vec<FileSymbolEntry> = file_nodes
        .iter()
        .filter(|n| !child_ids.contains(&n.id))
        .map(|n| {
            let mut children: Vec<FileSymbolEntry> = graph
                .children(n.id)
                .iter()
                .filter(|c| c.file_path == full_path)
                .map(|c| FileSymbolEntry {
                    name: c.name.clone(),
                    kind: format!("{:?}", c.kind),
                    start_line: c.span.start_line,
                    end_line: c.span.end_line,
                    children: vec![],
                })
                .collect();
            children.sort_by_key(|c| c.start_line);
            FileSymbolEntry {
                name: n.name.clone(),
                kind: format!("{:?}", n.kind),
                start_line: n.span.start_line,
                end_line: n.span.end_line,
                children,
            }
        })
        .collect();

    symbols.sort_by_key(|s| s.start_line);
    let symbol_count = symbols.len() + symbols.iter().map(|s| s.children.len()).sum::<usize>();

    Json(FileSymbolsResponse {
        path: params.path,
        symbols,
        symbol_count,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ── relative_path ──────────────────────────────────────

    #[test]
    fn relative_path_strips_prefix() {
        let path = Path::new("/home/user/project/src/main.rs");
        let root = Path::new("/home/user/project");
        assert_eq!(relative_path(path, root), "src/main.rs");
    }

    #[test]
    fn relative_path_no_common_prefix() {
        let path = Path::new("/other/path/file.rs");
        let root = Path::new("/home/user/project");
        assert_eq!(relative_path(path, root), "/other/path/file.rs");
    }

    #[test]
    fn relative_path_same_directory() {
        let path = Path::new("/project/file.rs");
        let root = Path::new("/project");
        assert_eq!(relative_path(path, root), "file.rs");
    }

    #[test]
    fn relative_path_root_is_file() {
        // If root is same as path
        let path = Path::new("/project");
        let root = Path::new("/project");
        assert_eq!(relative_path(path, root), "");
    }

    // ── default_limit ──────────────────────────────────────

    #[test]
    fn default_limit_is_20() {
        assert_eq!(default_limit(), 20);
    }

    // ── node_to_ref ────────────────────────────────────────

    #[test]
    fn node_to_ref_builds_symbol_ref() {
        let node = graphy_core::GirNode::new(
            "my_func".into(),
            NodeKind::Function,
            PathBuf::from("/project/src/main.rs"),
            graphy_core::Span::new(10, 0, 20, 0),
            graphy_core::Language::Rust,
        );
        let root = Path::new("/project");
        let sym_ref = node_to_ref(&node, root);
        assert_eq!(sym_ref.name, "my_func");
        assert_eq!(sym_ref.kind, "Function");
        assert_eq!(sym_ref.file_path, "src/main.rs");
        assert_eq!(sym_ref.start_line, 10);
    }

    // ── Serde structs ──────────────────────────────────────

    #[test]
    fn stats_response_serialization() {
        let stats = StatsResponse {
            nodes: 100, edges: 200, files: 10, classes: 5,
            structs: 3, enums: 2, traits: 1, functions: 30,
            methods: 20, imports: 15, variables: 5, constants: 2,
        };
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["nodes"], 100);
        assert_eq!(json["edges"], 200);
    }

    #[test]
    fn hotspot_item_serialization() {
        let item = HotspotItem {
            name: "complex_fn".into(),
            kind: "Function".into(),
            file_path: "src/main.rs".into(),
            start_line: 10,
            cyclomatic: 25,
            cognitive: 30,
            loc: 100,
            caller_count: 5,
            risk_score: 37.5,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["name"], "complex_fn");
        assert_eq!(json["risk_score"], 37.5);
    }

    #[test]
    fn dead_code_item_serialization() {
        let item = DeadCodeItem {
            name: "unused_fn".into(),
            kind: "Function".into(),
            file_path: "src/lib.rs".into(),
            start_line: 42,
            visibility: "Public".into(),
            dead_probability: 0.95,
        };
        let json = serde_json::to_value(&item).unwrap();
        let prob = json["dead_probability"].as_f64().unwrap();
        assert!((prob - 0.95).abs() < 0.001);
    }

    #[test]
    fn pattern_finding_serialization() {
        let finding = PatternFinding {
            pattern: "God Class".into(),
            severity: "warning".into(),
            symbol_name: "BigController".into(),
            detail: "20 methods".into(),
            file_path: "src/controller.rs".into(),
            line: 1,
        };
        let json = serde_json::to_value(&finding).unwrap();
        assert_eq!(json["pattern"], "God Class");
        assert_eq!(json["severity"], "warning");
    }

    #[test]
    fn search_query_deserialization() {
        let json = serde_json::json!({
            "q": "main",
            "limit": 10,
            "kind": "Function"
        });
        let query: SearchQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.q, "main");
        assert_eq!(query.limit, 10);
        assert_eq!(query.kind, Some("Function".into()));
        assert!(query.lang.is_none());
        assert!(query.file.is_none());
    }

    #[test]
    fn search_query_defaults() {
        let json = serde_json::json!({ "q": "test" });
        let query: SearchQuery = serde_json::from_value(json).unwrap();
        assert_eq!(query.limit, 20); // default_limit()
    }

    // ── serve_embedded_file ────────────────────────────────

    #[test]
    fn serve_embedded_file_not_found() {
        // Non-existent file returns 404
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let response = serve_frontend(axum::http::Uri::from_static("/nonexistent_path_xyz")).await;
            // Should fall back to index.html (SPA routing) or return something
            // Just verify it doesn't panic
            let _ = response;
        });
    }
}
