use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use graphy_core::{CodeGraph, EdgeKind, NodeKind, Visibility};
use graphy_search::SearchIndex;

use crate::protocol::*;
use crate::tools::{self, SessionContext};

/// MCP Server that communicates over stdio using JSON-RPC 2.0.
pub struct McpServer {
    graph: Arc<RwLock<CodeGraph>>,
    search: Option<Arc<SearchIndex>>,
    project_root: PathBuf,
    session: SessionContext,
    /// Channel to receive graph-updated notifications from watch mode.
    notify_rx: Option<tokio::sync::mpsc::Receiver<GraphUpdateEvent>>,
}

/// Event emitted by the file watcher when the graph is updated.
#[derive(Debug, Clone)]
pub struct GraphUpdateEvent {
    pub files_changed: usize,
    pub node_count: usize,
    pub edge_count: usize,
}

/// Sender half — give this to the file watcher.
pub type GraphUpdateNotifier = tokio::sync::mpsc::Sender<GraphUpdateEvent>;

/// Create a notification channel pair.
pub fn notification_channel() -> (GraphUpdateNotifier, tokio::sync::mpsc::Receiver<GraphUpdateEvent>) {
    tokio::sync::mpsc::channel(16)
}

impl McpServer {
    /// Create from owned values (wraps in Arc/RwLock internally).
    pub fn new(graph: CodeGraph, search: Option<SearchIndex>, project_root: PathBuf) -> Self {
        Self {
            graph: Arc::new(RwLock::new(graph)),
            search: search.map(Arc::new),
            project_root,
            session: SessionContext::new(),
            notify_rx: None,
        }
    }

    /// Create from pre-shared Arc<RwLock<CodeGraph>> — for watch mode hot reload.
    pub fn new_shared(
        graph: Arc<RwLock<CodeGraph>>,
        search: Option<Arc<SearchIndex>>,
        project_root: PathBuf,
    ) -> Self {
        Self {
            graph,
            search,
            project_root,
            session: SessionContext::new(),
            notify_rx: None,
        }
    }

    /// Attach a notification receiver for graph updates (from watch mode).
    pub fn with_notifications(
        mut self,
        rx: tokio::sync::mpsc::Receiver<GraphUpdateEvent>,
    ) -> Self {
        self.notify_rx = Some(rx);
        self
    }

    /// Run the MCP server, reading from stdin and writing to stdout.
    pub async fn run(mut self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        info!("Graphy MCP server started (stdio transport)");

        // If we have a notification channel, run both stdin and notifications concurrently
        if let Some(mut notify_rx) = self.notify_rx.take() {
            loop {
                tokio::select! {
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                let line = line.trim().to_string();
                                if line.is_empty() {
                                    continue;
                                }
                                debug!("Received: {}", &line[..line.len().min(200)]);

                                let request: JsonRpcRequest = match serde_json::from_str(&line) {
                                    Ok(req) => req,
                                    Err(e) => {
                                        let resp = JsonRpcResponse::error(
                                            None, -32700, format!("Parse error: {e}"),
                                        );
                                        let out = serde_json::to_string(&resp)? + "\n";
                                        stdout.write_all(out.as_bytes()).await?;
                                        stdout.flush().await?;
                                        continue;
                                    }
                                };

                                let response = self.handle_request(&request).await;
                                if let Some(resp) = response {
                                    let out = serde_json::to_string(&resp)? + "\n";
                                    debug!("Sending: {}", &out[..out.len().min(200)]);
                                    stdout.write_all(out.as_bytes()).await?;
                                    stdout.flush().await?;
                                }
                            }
                            Ok(None) => break, // stdin closed
                            Err(e) => {
                                error!("stdin read error: {e}");
                                break;
                            }
                        }
                    }
                    Some(event) = notify_rx.recv() => {
                        let notification = JsonRpcNotification::new(
                            "notifications/resources/updated",
                            Some(serde_json::json!({
                                "uri": "graphy://architecture",
                                "meta": {
                                    "filesChanged": event.files_changed,
                                    "nodes": event.node_count,
                                    "edges": event.edge_count,
                                }
                            })),
                        );
                        if let Ok(out) = serde_json::to_string(&notification) {
                            let out = out + "\n";
                            debug!("Sending notification: graph updated");
                            let _ = stdout.write_all(out.as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                    }
                }
            }
        } else {
            // Simple mode: just read stdin
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                debug!("Received: {}", &line[..line.len().min(200)]);

                let request: JsonRpcRequest = match serde_json::from_str(&line) {
                    Ok(req) => req,
                    Err(e) => {
                        let resp = JsonRpcResponse::error(
                            None, -32700, format!("Parse error: {e}"),
                        );
                        let out = serde_json::to_string(&resp)? + "\n";
                        stdout.write_all(out.as_bytes()).await?;
                        stdout.flush().await?;
                        continue;
                    }
                };

                let response = self.handle_request(&request).await;
                if let Some(resp) = response {
                    let out = serde_json::to_string(&resp)? + "\n";
                    debug!("Sending: {}", &out[..out.len().min(200)]);
                    stdout.write_all(out.as_bytes()).await?;
                    stdout.flush().await?;
                }
            }
        }

        info!("MCP server shutting down");
        Ok(())
    }

    async fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        match req.method.as_str() {
            "initialize" => Some(self.handle_initialize(req.id.clone())),
            "notifications/initialized" => None,
            "tools/list" => Some(self.handle_tools_list(req.id.clone())),
            "tools/call" => Some(self.handle_tools_call(req.id.clone(), &req.params).await),
            "resources/list" => Some(self.handle_resources_list(req.id.clone())),
            "resources/read" => Some(self.handle_resources_read(req.id.clone(), &req.params).await),
            "ping" => Some(JsonRpcResponse::success(
                req.id.clone(),
                serde_json::json!({}),
            )),
            method => {
                error!("Unknown method: {method}");
                Some(JsonRpcResponse::error(
                    req.id.clone(),
                    -32601,
                    format!("Method not found: {method}"),
                ))
            }
        }
    }

    fn handle_initialize(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let result = InitializeResult {
            protocol_version: "2024-11-05".into(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {}),
                resources: Some(ResourcesCapability {}),
            },
            server_info: ServerInfo {
                name: "graphy".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };

        match serde_json::to_value(result) {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err(e) => JsonRpcResponse::error(id, -32603, format!("Serialization failed: {e}")),
        }
    }

    fn handle_tools_list(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let result = ToolsListResult {
            tools: tools::tool_definitions(),
        };
        match serde_json::to_value(result) {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err(e) => JsonRpcResponse::error(id, -32603, format!("Serialization failed: {e}")),
        }
    }

    async fn handle_tools_call(
        &self,
        id: Option<serde_json::Value>,
        params: &serde_json::Value,
    ) -> JsonRpcResponse {
        let call_params: CallToolParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid params: {e}"),
                );
            }
        };

        let graph = self.graph.read().await;
        let result = tools::handle_tool(
            &call_params.name,
            &call_params.arguments,
            &graph,
            self.search.as_deref(),
            &self.project_root,
            &self.session,
        );

        match serde_json::to_value(result) {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err(e) => JsonRpcResponse::error(id, -32603, format!("Serialization failed: {e}")),
        }
    }

    // ── Resources ───────────────────────────────────────────

    fn handle_resources_list(&self, id: Option<serde_json::Value>) -> JsonRpcResponse {
        let result = ResourcesListResult {
            resources: vec![
                ResourceDefinition {
                    uri: "graphy://architecture".into(),
                    name: "Architecture Overview".into(),
                    description: Some(
                        "Codebase structure: file count, language breakdown, largest modules, entry points"
                            .into(),
                    ),
                    mime_type: Some("text/markdown".into()),
                },
                ResourceDefinition {
                    uri: "graphy://security".into(),
                    name: "Security Summary".into(),
                    description: Some(
                        "Taint analysis paths, public API exposure, vulnerability findings"
                            .into(),
                    ),
                    mime_type: Some("text/markdown".into()),
                },
                ResourceDefinition {
                    uri: "graphy://health".into(),
                    name: "Codebase Health".into(),
                    description: Some(
                        "Dead code count, complexity hotspots, anti-patterns, graph confidence"
                            .into(),
                    ),
                    mime_type: Some("text/markdown".into()),
                },
            ],
        };
        match serde_json::to_value(result) {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err(e) => JsonRpcResponse::error(id, -32603, format!("Serialization failed: {e}")),
        }
    }

    async fn handle_resources_read(
        &self,
        id: Option<serde_json::Value>,
        params: &serde_json::Value,
    ) -> JsonRpcResponse {
        let read_params: ResourceReadParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid params: {e}"),
                );
            }
        };

        let graph = self.graph.read().await;
        let text = match read_params.uri.as_str() {
            "graphy://architecture" => self.resource_architecture(&graph),
            "graphy://security" => self.resource_security(&graph),
            "graphy://health" => self.resource_health(&graph),
            uri => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Unknown resource: {uri}"),
                );
            }
        };

        let result = ResourceReadResult {
            contents: vec![ResourceContent {
                uri: read_params.uri,
                mime_type: Some("text/markdown".into()),
                text,
            }],
        };

        match serde_json::to_value(result) {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err(e) => JsonRpcResponse::error(id, -32603, format!("Serialization failed: {e}")),
        }
    }

    fn resource_architecture(&self, graph: &CodeGraph) -> String {
        let files = graph.find_by_kind(NodeKind::File);
        let classes = graph.find_by_kind(NodeKind::Class);
        let functions = graph.find_by_kind(NodeKind::Function);
        let methods = graph.find_by_kind(NodeKind::Method);
        let structs = graph.find_by_kind(NodeKind::Struct);
        let imports = graph.find_by_kind(NodeKind::Import);

        let mut out = format!(
            "# Architecture Overview\n\n\
             | Metric | Count |\n|--------|-------|\n\
             | Files | {} |\n| Classes | {} |\n| Structs | {} |\n\
             | Functions | {} |\n| Methods | {} |\n| Imports | {} |\n\
             | Total nodes | {} |\n| Total edges | {} |\n\n",
            files.len(), classes.len(), structs.len(),
            functions.len(), methods.len(), imports.len(),
            graph.node_count(), graph.edge_count(),
        );

        // Language breakdown
        let mut lang_counts: HashMap<String, usize> = HashMap::new();
        for node in graph.all_nodes() {
            *lang_counts.entry(format!("{:?}", node.language)).or_default() += 1;
        }
        let mut lang_sorted: Vec<_> = lang_counts.into_iter().collect();
        lang_sorted.sort_by(|a, b| b.1.cmp(&a.1));

        out.push_str("## Languages\n\n");
        for (lang, count) in &lang_sorted {
            out.push_str(&format!("- **{lang}**: {count} symbols\n"));
        }

        // Largest files
        let mut file_counts: HashMap<&std::path::Path, usize> = HashMap::new();
        for node in graph.all_nodes() {
            *file_counts.entry(&node.file_path).or_default() += 1;
        }
        let mut sorted: Vec<_> = file_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        out.push_str("\n## Largest Files (by symbol count)\n\n");
        for (path, count) in sorted.iter().take(15) {
            out.push_str(&format!("- `{}` — {} symbols\n", path.display(), count));
        }

        // Entry points
        let entries: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && n.visibility == Visibility::Public)
            .filter(|n| graph.callers(n.id).is_empty())
            .take(20)
            .collect();
        if !entries.is_empty() {
            out.push_str(&format!(
                "\n## Entry Points ({} public, uncalled)\n\n",
                entries.len()
            ));
            for e in &entries {
                out.push_str(&format!(
                    "- `{}` ({:?}) at {}:{}\n",
                    e.name, e.kind, e.file_path.display(), e.span.start_line
                ));
            }
        }

        out
    }

    fn resource_security(&self, graph: &CodeGraph) -> String {
        let mut out = String::from("# Security Summary\n\n");

        // Taint paths
        let tainted: Vec<_> = graph
            .all_nodes()
            .filter(|n| !graph.outgoing(n.id, EdgeKind::TaintedBy).is_empty())
            .collect();

        out.push_str(&format!("## Taint Analysis ({} tainted symbols)\n\n", tainted.len()));

        if tainted.is_empty() {
            out.push_str("No taint paths detected.\n\n");
        } else {
            for node in tainted.iter().take(30) {
                let sources = graph.outgoing(node.id, EdgeKind::TaintedBy);
                out.push_str(&format!(
                    "- **`{}`** at {}:{}\n",
                    node.name, node.file_path.display(), node.span.start_line
                ));
                for src in sources.iter().take(5) {
                    out.push_str(&format!("  - Tainted by: `{}`\n", src.name));
                }
            }
            if tainted.len() > 30 {
                out.push_str(&format!("\n... and {} more\n", tainted.len() - 30));
            }
        }

        // Public API surface
        let public_api: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && n.visibility == Visibility::Public)
            .filter(|n| {
                graph.callers(n.id).iter().any(|c| c.file_path != n.file_path)
            })
            .collect();
        let internal: usize = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && n.visibility != Visibility::Public)
            .count();

        out.push_str(&format!(
            "\n## API Surface\n\n- **Public API**: {} symbols\n- **Internal**: {} symbols\n",
            public_api.len(), internal
        ));

        out
    }

    fn resource_health(&self, graph: &CodeGraph) -> String {
        let mut out = String::from("# Codebase Health\n\n");

        // Dead code
        let dead_count = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && n.confidence < 0.5)
            .filter(|n| !graph.is_phantom(n.id))
            .count();
        let total_callables = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable())
            .count();

        out.push_str(&format!(
            "## Dead Code\n\n- **{dead_count}** likely dead functions out of {total_callables} total ({:.1}%)\n\n",
            if total_callables > 0 { dead_count as f64 / total_callables as f64 * 100.0 } else { 0.0 }
        ));

        // Top complexity hotspots
        let mut hotspots: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable())
            .filter_map(|n| {
                let cx = n.complexity.as_ref()?;
                let callers = graph.callers(n.id).len() as f64;
                let risk = cx.cyclomatic as f64 * (1.0 + callers * 0.5);
                Some((n, risk, cx.cyclomatic, cx.cognitive))
            })
            .collect();
        hotspots.sort_by(|a, b| b.1.total_cmp(&a.1));
        hotspots.truncate(10);

        if !hotspots.is_empty() {
            out.push_str("## Top 10 Complexity Hotspots\n\n");
            out.push_str("| Risk | Symbol | Cyclomatic | Cognitive | Location |\n");
            out.push_str("|------|--------|-----------|-----------|----------|\n");
            for (n, risk, cyc, cog) in &hotspots {
                out.push_str(&format!(
                    "| {:.1} | `{}` | {} | {} | {}:{} |\n",
                    risk, n.name, cyc, cog, n.file_path.display(), n.span.start_line
                ));
            }
        }

        // Graph confidence
        let with_callers = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable() && !graph.callers(n.id).is_empty())
            .count();
        let total_imports = graph.find_by_kind(NodeKind::Import).len();
        let resolved_imports = graph
            .all_nodes()
            .filter(|n| n.kind == NodeKind::Import)
            .filter(|n| !graph.outgoing(n.id, EdgeKind::ImportsFrom).is_empty())
            .count();

        let call_pct = if total_callables > 0 {
            (with_callers as f64 / total_callables as f64 * 100.0) as u32
        } else {
            0
        };
        let import_pct = if total_imports > 0 {
            (resolved_imports as f64 / total_imports as f64 * 100.0) as u32
        } else {
            100
        };

        out.push_str(&format!(
            "\n## Graph Confidence\n\n\
             - **Call coverage**: {}% ({} of {} callables have callers)\n\
             - **Import resolution**: {}% ({}/{})\n\
             - **Total**: {} nodes, {} edges\n",
            call_pct, with_callers, total_callables,
            import_pct, resolved_imports, total_imports,
            graph.node_count(), graph.edge_count(),
        ));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, Language, Span};

    fn make_fn(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    fn make_import(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Import,
            PathBuf::from(file),
            Span::new(line, 0, line, 30),
            Language::Python,
        )
    }

    #[test]
    fn server_new_creates_instance() {
        let graph = CodeGraph::new();
        let server = McpServer::new(graph, None, PathBuf::from("/tmp"));
        // Just confirm it constructs without panic
        drop(server);
    }

    #[test]
    fn server_new_shared_creates_instance() {
        let graph = Arc::new(RwLock::new(CodeGraph::new()));
        let server = McpServer::new_shared(graph, None, PathBuf::from("/tmp"));
        drop(server);
    }

    #[test]
    fn notification_channel_works() {
        let (tx, rx) = notification_channel();
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"))
            .with_notifications(rx);
        drop(tx);
        drop(server);
    }

    #[test]
    fn handle_initialize_returns_protocol_version() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_initialize(Some(serde_json::json!(1)));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "graphy");
    }

    #[test]
    fn handle_initialize_null_id() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_initialize(None);
        assert!(resp.error.is_none());
        assert!(resp.id.is_none());
    }

    #[test]
    fn handle_tools_list_returns_three_tools() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_tools_list(Some(serde_json::json!(2)));
        assert!(resp.error.is_none());
        let tools = resp.result.unwrap();
        assert_eq!(tools["tools"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn handle_resources_list_returns_three_resources() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_resources_list(Some(serde_json::json!(3)));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let resources = result["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 3);

        let uris: Vec<&str> = resources.iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert!(uris.contains(&"graphy://architecture"));
        assert!(uris.contains(&"graphy://security"));
        assert!(uris.contains(&"graphy://health"));
    }

    #[tokio::test]
    async fn handle_request_ping() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "ping".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(42)),
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn handle_request_unknown_method() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "bogus/method".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(1)),
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    }

    #[tokio::test]
    async fn handle_request_notifications_initialized_returns_none() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: serde_json::json!({}),
            id: None,
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn handle_tools_call_invalid_params() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_tools_call(
            Some(serde_json::json!(1)),
            &serde_json::json!("not_an_object"),
        ).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.as_ref().unwrap().code, -32602);
    }

    #[tokio::test]
    async fn handle_tools_call_valid_query() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_tools_call(
            Some(serde_json::json!(1)),
            &serde_json::json!({
                "name": "graphy_query",
                "arguments": {"query": "test", "mode": "search"}
            }),
        ).await;
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn handle_resources_read_architecture() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_resources_read(
            Some(serde_json::json!(1)),
            &serde_json::json!({"uri": "graphy://architecture"}),
        ).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let text = result["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("Architecture Overview"));
    }

    #[tokio::test]
    async fn handle_resources_read_security() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_resources_read(
            Some(serde_json::json!(1)),
            &serde_json::json!({"uri": "graphy://security"}),
        ).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let text = result["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("Security Summary"));
    }

    #[tokio::test]
    async fn handle_resources_read_health() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_resources_read(
            Some(serde_json::json!(1)),
            &serde_json::json!({"uri": "graphy://health"}),
        ).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let text = result["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("Codebase Health"));
    }

    #[tokio::test]
    async fn handle_resources_read_unknown_uri() {
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let resp = server.handle_resources_read(
            Some(serde_json::json!(1)),
            &serde_json::json!({"uri": "graphy://nonexistent"}),
        ).await;
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("Unknown resource"));
    }

    #[test]
    fn resource_architecture_with_data() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn("main", "app.py", 1));
        graph.add_node(make_import("os", "app.py", 1));

        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let text = server.resource_architecture(&graph);
        assert!(text.contains("Architecture Overview"));
        assert!(text.contains("Languages"));
    }

    #[test]
    fn resource_security_with_taint() {
        let mut graph = CodeGraph::new();
        let src = make_fn("input", "a.py", 1);
        let sink = make_fn("query", "a.py", 10);
        let src_id = src.id;
        let sink_id = sink.id;
        graph.add_node(src);
        graph.add_node(sink);
        graph.add_edge(sink_id, src_id, graphy_core::GirEdge::new(EdgeKind::TaintedBy));

        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let text = server.resource_security(&graph);
        assert!(text.contains("1 tainted symbols"));
        assert!(text.contains("query"));
    }

    #[test]
    fn resource_health_empty_graph() {
        let graph = CodeGraph::new();
        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let text = server.resource_health(&graph);
        assert!(text.contains("Codebase Health"));
        assert!(text.contains("**0** likely dead"));
    }

    #[test]
    fn resource_health_with_dead_code() {
        let mut graph = CodeGraph::new();
        // Need a file parent so the function isn't phantom
        let file = GirNode::new(
            "a.py".to_string(), NodeKind::File, PathBuf::from("a.py"),
            Span::new(0, 0, 100, 0), Language::Python,
        );
        let file_id = file.id;
        graph.add_node(file);

        let mut f = make_fn("dead", "a.py", 1);
        f.confidence = 0.1;
        let f_id = f.id;
        graph.add_node(f);
        graph.add_edge(file_id, f_id, graphy_core::GirEdge::new(EdgeKind::Contains));

        let server = McpServer::new(CodeGraph::new(), None, PathBuf::from("/tmp"));
        let text = server.resource_health(&graph);
        assert!(text.contains("**1** likely dead"));
    }

    #[test]
    fn graph_update_event_debug() {
        let event = GraphUpdateEvent {
            files_changed: 5,
            node_count: 100,
            edge_count: 200,
        };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("files_changed: 5"));
    }
}
