//! Optional LSP Enhancement Phase.
//!
//! Queries installed language servers (rust-analyzer, pyright, gopls, etc.)
//! for precise reference and call hierarchy data, then adds edges to the graph.
//!
//! This replaces tree-sitter heuristics with compiler-grade precision for
//! languages where an LSP server is available. Falls back gracefully when not.
//!
//! Architecture:
//! - Language-agnostic LSP client (JSON-RPC over stdio, same as MCP)
//! - Detects available servers by checking PATH
//! - Queries `callHierarchy/incomingCalls` for each function
//! - Pipelined requests: sends batches of 50 in parallel for 10-20x speedup
//! - Falls back to `textDocument/references` when call hierarchy returns empty
//! - Sends `textDocument/didOpen` for pyright/TS compatibility
//! - Adds precise `Calls` edges to the graph
//! - Completely optional — tree-sitter analysis works without it

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
use graphy_core::{CodeGraph, EdgeKind, EdgeMetadata, GirEdge, Language, NodeKind, SymbolId};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

/// Known LSP servers and the languages they handle.
/// The binary names are checked on PATH — no hardcoded paths.
const LSP_SERVERS: &[(&str, &[Language])] = &[
    ("rust-analyzer", &[Language::Rust]),
    ("pyright-langserver", &[Language::Python]),
    (
        "typescript-language-server",
        &[Language::TypeScript, Language::JavaScript],
    ),
    ("gopls", &[Language::Go]),
];

/// Result of an LSP enhancement pass.
#[derive(Debug, Default)]
pub struct LspEnhanceResult {
    pub servers_used: Vec<String>,
    pub edges_added: usize,
    pub functions_queried: usize,
}

/// Map Language to the LSP language identifier string.
fn language_to_lsp_id(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::TypeScript => "typescript",
        Language::JavaScript => "javascript",
        Language::Rust => "rust",
        Language::Go => "go",
        Language::Java => "java",
        Language::Cpp => "cpp",
        Language::C => "c",
        Language::CSharp => "csharp",
        Language::Ruby => "ruby",
        Language::Kotlin => "kotlin",
        Language::Php => "php",
        Language::Svelte => "svelte",
    }
}

/// Run LSP enhancement: detect available servers, query references, add edges.
pub fn enhance_with_lsp(graph: &mut CodeGraph, root: &Path) -> LspEnhanceResult {
    let mut result = LspEnhanceResult::default();

    // Detect which languages are in the graph
    let languages_present: Vec<Language> = graph
        .all_nodes()
        .filter(|n| n.kind == NodeKind::File)
        .map(|n| n.language)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    for (server_bin, langs) in LSP_SERVERS {
        // Skip if none of this server's languages are in the project
        if !langs.iter().any(|l| languages_present.contains(l)) {
            continue;
        }

        // Check if the server binary is on PATH
        if !is_on_path(server_bin) {
            debug!(
                "LSP server '{}' not found on PATH, skipping",
                server_bin
            );
            continue;
        }

        info!("Starting LSP server: {}", server_bin);
        match LspClient::start(server_bin, root) {
            Ok(mut client) => {
                let edges = query_incoming_calls(&mut client, graph, root, langs);
                result.edges_added += edges;
                result.functions_queried += count_functions(graph, langs);
                result.servers_used.push(server_bin.to_string());

                if let Err(e) = client.shutdown() {
                    debug!("LSP shutdown error (non-fatal): {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to start LSP server '{}': {}", server_bin, e);
            }
        }
    }

    result
}

/// LSP client that communicates over stdio using JSON-RPC 2.0.
/// Uses a background reader thread so reads never block the timeout logic.
pub struct LspClient {
    process: Child,
    next_id: i64,
    receiver: std::sync::mpsc::Receiver<Value>,
    /// Files opened via textDocument/didOpen (for didClose on shutdown).
    opened_files: HashSet<PathBuf>,
    /// Whether the server advertised callHierarchyProvider capability.
    supports_call_hierarchy: bool,
}

impl LspClient {
    pub fn start(server_bin: &str, root: &Path) -> Result<Self> {
        let args: Vec<&str> = match server_bin {
            "pyright-langserver" => vec!["--stdio"],
            "typescript-language-server" => vec!["--stdio"],
            _ => vec![],
        };

        let mut process = Command::new(server_bin)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn {}", server_bin))?;

        // Take stdout and move it to a background reader thread.
        let stdout = process.stdout.take().context("stdout not available")?;
        let (tx, rx) = std::sync::mpsc::channel::<Value>();

        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.is_empty() {
                    break;
                }
                if header.trim().is_empty() {
                    continue;
                }

                let content_length: usize = header
                    .trim()
                    .strip_prefix("Content-Length: ")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                if content_length == 0 {
                    continue;
                }

                let mut blank = String::new();
                if reader.read_line(&mut blank).is_err() {
                    break;
                }

                let mut body = vec![0u8; content_length];
                if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                    break;
                }

                if let Ok(msg) = serde_json::from_slice::<Value>(&body) {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
            }
        });

        let mut client = Self {
            process,
            next_id: 1,
            receiver: rx,
            opened_files: HashSet::new(),
            supports_call_hierarchy: true, // assume true until proven otherwise
        };

        // Send initialize request with progress support.
        let root_uri = format!("file://{}", root.display());
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "callHierarchy": {
                        "dynamicRegistration": false
                    },
                    "references": {
                        "dynamicRegistration": false
                    }
                },
                "window": {
                    "workDoneProgress": true
                }
            }
        });

        let init_result = client.request_with_timeout("initialize", init_params, 60)?;

        // Check if server advertises callHierarchyProvider
        if let Some(caps) = init_result.get("capabilities") {
            if caps.get("callHierarchyProvider").is_some() {
                client.supports_call_hierarchy = true;
            } else {
                client.supports_call_hierarchy = false;
                debug!("{}: no callHierarchyProvider, will use references fallback", server_bin);
            }
        }

        client.notify("initialized", json!({}))?;

        // Wait for the server to finish indexing.
        client.wait_for_indexing(120)?;

        Ok(client)
    }

    /// Wait for the LSP server to finish indexing by tracking all active progress tokens.
    fn wait_for_indexing(&mut self, timeout_secs: u64) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        let mut active_tokens = HashSet::<String>::new();
        let mut seen_any = false;

        info!("Waiting for LSP server to finish indexing (up to {}s)...", timeout_secs);

        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                warn!("LSP indexing timed out after {}s, proceeding with {} active tasks",
                    timeout_secs, active_tokens.len());
                return Ok(());
            }

            let wait_time = if seen_any && active_tokens.is_empty() {
                std::time::Duration::from_secs(3)
            } else if !seen_any {
                std::cmp::min(remaining, std::time::Duration::from_secs(5))
            } else {
                std::cmp::min(remaining, std::time::Duration::from_secs(10))
            };

            match self.receiver.recv_timeout(wait_time) {
                Ok(msg) => {
                    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

                    if method == "window/workDoneProgress/create" {
                        if let Some(id) = msg.get("id") {
                            let response = json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": null
                            });
                            let _ = self.send(&response);
                        }
                        continue;
                    }

                    if method == "$/progress" {
                        let token = msg["params"]["token"].as_str()
                            .map(String::from)
                            .or_else(|| msg["params"]["token"].as_i64().map(|n| n.to_string()))
                            .unwrap_or_default();
                        let kind = msg["params"]["value"]["kind"].as_str().unwrap_or("");
                        let title = msg["params"]["value"]["title"].as_str().unwrap_or("");
                        let message = msg["params"]["value"]["message"].as_str().unwrap_or("");

                        match kind {
                            "begin" => {
                                seen_any = true;
                                active_tokens.insert(token.clone());
                                debug!("LSP progress begin [{}]: {} {}", token, title, message);
                            }
                            "report" => {
                                debug!("LSP progress [{}]: {} {}", token, title, message);
                            }
                            "end" => {
                                active_tokens.remove(&token);
                                debug!("LSP progress end [{}]: {} active remaining", token, active_tokens.len());
                            }
                            _ => {}
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if seen_any && active_tokens.is_empty() {
                        info!("LSP server finished indexing (all progress tokens ended)");
                        return Ok(());
                    }
                    if !seen_any {
                        info!("No progress notifications from LSP, proceeding");
                        return Ok(());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("LSP server disconnected during indexing");
                }
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(method, params, 10)
    }

    fn request_with_timeout(&mut self, method: &str, params: Value, timeout_secs: u64) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send(&msg)?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("LSP response timeout for request {}", id);
            }

            match self.receiver.recv_timeout(remaining) {
                Ok(response) => {
                    if response.get("id").is_none() {
                        continue;
                    }
                    if response["id"].as_i64() == Some(id) {
                        if let Some(error) = response.get("error") {
                            anyhow::bail!("LSP error: {}", error);
                        }
                        return Ok(response["result"].clone());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    anyhow::bail!("LSP response timeout for request {}", id);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("LSP server disconnected");
                }
            }
        }
    }

    /// Send a request without waiting for the response. Returns the request ID.
    fn send_request_async(&mut self, method: &str, params: Value) -> Result<i64> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send(&msg)?;
        Ok(id)
    }

    /// Collect responses for a set of request IDs. Returns map of id -> result.
    /// Skips notifications and server requests. Stops at timeout or when all IDs received.
    fn collect_responses(&mut self, ids: &[i64], timeout_secs: u64) -> HashMap<i64, Value> {
        let mut results = HashMap::new();
        if ids.is_empty() {
            return results;
        }
        let expected: HashSet<i64> = ids.iter().copied().collect();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        while results.len() < ids.len() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match self.receiver.recv_timeout(remaining) {
                Ok(msg) => {
                    // Handle server requests (workDoneProgress/create)
                    if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                        if method == "window/workDoneProgress/create" {
                            if let Some(req_id) = msg.get("id") {
                                let _ = self.send(&json!({
                                    "jsonrpc": "2.0",
                                    "id": req_id,
                                    "result": null
                                }));
                            }
                        }
                        continue;
                    }

                    // Check if this is a response we're waiting for
                    if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
                        if expected.contains(&id) {
                            if msg.get("error").is_some() {
                                results.insert(id, Value::Null);
                            } else {
                                results.insert(id, msg["result"].clone());
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        results
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send(&msg)
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        let body = serde_json::to_string(msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let stdin = self
            .process
            .stdin
            .as_mut()
            .context("stdin not available")?;
        stdin.write_all(header.as_bytes())?;
        stdin.write_all(body.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Open a file via textDocument/didOpen. Required for pyright and typescript-language-server.
    pub fn open_file(&mut self, path: &Path, language: Language) -> Result<()> {
        if self.opened_files.contains(path) {
            return Ok(());
        }
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let uri = format!("file://{}", path.display());
        self.notify("textDocument/didOpen", json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_to_lsp_id(language),
                "version": 1,
                "text": content
            }
        }))?;
        self.opened_files.insert(path.to_path_buf());
        Ok(())
    }

    /// Send textDocument/didChange for a previously opened file.
    pub fn did_change(&mut self, path: &Path, content: &str) -> Result<()> {
        let uri = format!("file://{}", path.display());
        if self.opened_files.contains(path) {
            self.notify("textDocument/didChange", json!({
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": content }]
            }))?;
        } else {
            // Auto-detect language and open
            let lang = path.extension()
                .and_then(|e| e.to_str())
                .and_then(Language::from_extension)
                .unwrap_or(Language::Python);
            self.open_file(path, lang)?;
        }
        Ok(())
    }

    /// Check if the LSP server process is still alive.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.process.try_wait(), Ok(None))
    }

    pub fn shutdown(&mut self) -> Result<()> {
        // Close all opened files
        let opened: Vec<PathBuf> = self.opened_files.drain().collect();
        for path in opened {
            let uri = format!("file://{}", path.display());
            let _ = self.notify("textDocument/didClose", json!({
                "textDocument": { "uri": uri }
            }));
        }

        let _ = self.request("shutdown", json!(null));
        let _ = self.notify("exit", json!(null));
        let _ = self.process.kill();
        let _ = self.process.wait();
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Query incoming calls for all functions in the graph that match the given languages.
/// Uses pipelined requests for 10-20x speedup over sequential queries.
fn query_incoming_calls(
    client: &mut LspClient,
    graph: &mut CodeGraph,
    _root: &Path,
    languages: &[Language],
) -> usize {
    let mut edges_added = 0;

    // Collect all functions to query (avoid borrowing graph while mutating)
    let targets: Vec<(SymbolId, String, PathBuf, u32, Language)> = graph
        .all_nodes()
        .filter(|n| {
            matches!(n.kind, NodeKind::Function | NodeKind::Method | NodeKind::Constructor)
                && languages.contains(&n.language)
                && !graph.is_phantom(n.id)
        })
        .map(|n| {
            (
                n.id,
                n.name.clone(),
                n.file_path.clone(),
                n.span.start_line,
                n.language,
            )
        })
        .collect();

    // Cache file contents to find name positions on each line.
    let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();

    // Phase 1: Open all unique files via textDocument/didOpen.
    // This is required for pyright and typescript-language-server,
    // which return empty results for files not opened in the editor.
    let mut file_langs: HashMap<PathBuf, Language> = HashMap::new();
    for (_, _, path, _, lang) in &targets {
        file_langs.entry(path.clone()).or_insert(*lang);
    }
    for (path, lang) in &file_langs {
        let _ = client.open_file(path, *lang);
    }

    // Track targets that got no incoming calls for references fallback
    let mut empty_targets: Vec<(SymbolId, String, PathBuf, u32)> = Vec::new();

    // Process in chunks for pipelined requests
    const CHUNK_SIZE: usize = 50;

    for chunk in targets.chunks(CHUNK_SIZE) {
        // Batch 1: Send all prepareCallHierarchy requests
        let mut prepare_requests: Vec<(i64, usize)> = Vec::new();

        for (idx, (_, name, file_path, line, _)) in chunk.iter().enumerate() {
            let uri = format!("file://{}", file_path.display());

            let col = find_name_column(&mut file_cache, file_path, *line, name);

            let params = json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col }
            });

            if let Ok(id) = client.send_request_async("textDocument/prepareCallHierarchy", params) {
                prepare_requests.push((id, idx));
            }
        }

        // Collect all prepare responses
        let req_ids: Vec<i64> = prepare_requests.iter().map(|(id, _)| *id).collect();
        let prepare_responses = client.collect_responses(&req_ids, 30);

        // Batch 2: Send all incomingCalls requests for functions that returned items
        let mut incoming_requests: Vec<(i64, usize)> = Vec::new();
        let mut prepared_items: HashMap<usize, Value> = HashMap::new();

        for (req_id, chunk_idx) in &prepare_requests {
            if let Some(result) = prepare_responses.get(req_id) {
                if let Some(items) = result.as_array() {
                    if !items.is_empty() {
                        prepared_items.insert(*chunk_idx, items[0].clone());
                        let params = json!({ "item": items[0] });
                        match client.send_request_async("callHierarchy/incomingCalls", params) {
                            Ok(id) => incoming_requests.push((id, *chunk_idx)),
                            Err(_) => {}
                        }
                    }
                }
            }
        }

        // Collect all incoming call responses
        let req_ids: Vec<i64> = incoming_requests.iter().map(|(id, _)| *id).collect();
        let incoming_responses = client.collect_responses(&req_ids, 30);

        // Process results and track empty targets for references fallback
        let incoming_idx_set: HashSet<usize> = incoming_requests.iter().map(|(_, idx)| *idx).collect();

        for (req_id, chunk_idx) in &incoming_requests {
            let (sym_id, name, file_path, _, _) = &chunk[*chunk_idx];

            if let Some(result) = incoming_responses.get(req_id) {
                if let Some(calls) = result.as_array() {
                    if calls.is_empty() {
                        empty_targets.push((*sym_id, name.clone(), file_path.clone(), chunk[*chunk_idx].3));
                        continue;
                    }
                    for call in calls {
                        edges_added += process_incoming_call(graph, call, *sym_id, name, file_path);
                    }
                    continue;
                }
            }
            // No response or null result — treat as empty
            empty_targets.push((*sym_id, name.clone(), file_path.clone(), chunk[*chunk_idx].3));
        }

        // Also track targets that didn't even get a prepare response
        for (_, chunk_idx) in &prepare_requests {
            if !incoming_idx_set.contains(chunk_idx) {
                let (sym_id, name, file_path, line, _) = &chunk[*chunk_idx];
                empty_targets.push((*sym_id, name.clone(), file_path.clone(), *line));
            }
        }
    }

    // Phase 3: References fallback for targets with no incoming calls.
    // Uses textDocument/references to find call sites, then maps to containing functions.
    if !empty_targets.is_empty() {
        let ref_edges = query_references_fallback(client, graph, &empty_targets, &mut file_cache);
        edges_added += ref_edges;
        if ref_edges > 0 {
            debug!("LSP references fallback: {} additional edges from {} empty targets",
                ref_edges, empty_targets.len());
        }
    }

    edges_added
}

/// Process a single incoming call result and add edge to graph.
fn process_incoming_call(
    graph: &mut CodeGraph,
    call: &Value,
    sym_id: SymbolId,
    name: &str,
    file_path: &Path,
) -> usize {
    let from = &call["from"];
    let caller_name = from["name"].as_str().unwrap_or("");
    let caller_uri = from["uri"].as_str().unwrap_or("");
    let caller_line = from["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;

    let caller_path = uri_to_path(caller_uri);

    let caller_id = graph
        .all_nodes()
        .find(|n| {
            n.file_path == caller_path
                && n.name == caller_name
                && n.span.start_line == caller_line
        })
        .map(|n| n.id);

    if let Some(caller_id) = caller_id {
        let already_exists = graph
            .callers(sym_id)
            .iter()
            .any(|c| c.id == caller_id);

        if !already_exists {
            graph.add_edge(
                caller_id,
                sym_id,
                GirEdge::new(EdgeKind::Calls)
                    .with_confidence(1.0)
                    .with_metadata(EdgeMetadata::Call { is_dynamic: false }),
            );
            debug!("LSP: {} -> {} ({})", caller_name, name, file_path.display());
            return 1;
        }
    }
    0
}

/// Fallback: use textDocument/references to find call sites for functions
/// that returned no incoming calls from the call hierarchy.
fn query_references_fallback(
    client: &mut LspClient,
    graph: &mut CodeGraph,
    targets: &[(SymbolId, String, PathBuf, u32)],
    file_cache: &mut HashMap<PathBuf, Vec<String>>,
) -> usize {
    let mut edges_added = 0;
    const CHUNK_SIZE: usize = 50;

    for chunk in targets.chunks(CHUNK_SIZE) {
        let mut ref_requests: Vec<(i64, usize)> = Vec::new();

        for (idx, (_, name, file_path, line)) in chunk.iter().enumerate() {
            let uri = format!("file://{}", file_path.display());

            let col = find_name_column(file_cache, file_path, *line, name);

            let params = json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col },
                "context": { "includeDeclaration": false }
            });

            if let Ok(id) = client.send_request_async("textDocument/references", params) {
                ref_requests.push((id, idx));
            }
        }

        let req_ids: Vec<i64> = ref_requests.iter().map(|(id, _)| *id).collect();
        let responses = client.collect_responses(&req_ids, 30);

        for (req_id, chunk_idx) in &ref_requests {
            let (sym_id, _name, _file_path, _) = &chunk[*chunk_idx];

            if let Some(result) = responses.get(req_id) {
                if let Some(locations) = result.as_array() {
                    for loc in locations {
                        let ref_uri = loc["uri"].as_str().unwrap_or("");
                        let ref_line = loc["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;

                        let ref_path = uri_to_path(ref_uri);

                        // Find the function that contains this reference
                        if let Some(caller_id) = find_containing_function(graph, &ref_path, ref_line) {
                            if caller_id != *sym_id {
                                let already_exists = graph
                                    .callers(*sym_id)
                                    .iter()
                                    .any(|c| c.id == caller_id);

                                if !already_exists {
                                    graph.add_edge(
                                        caller_id,
                                        *sym_id,
                                        GirEdge::new(EdgeKind::Calls)
                                            .with_confidence(0.7)
                                            .with_metadata(EdgeMetadata::Call { is_dynamic: false }),
                                    );
                                    edges_added += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    edges_added
}

/// Find the narrowest function/method/constructor whose span contains the given line.
fn find_containing_function(graph: &CodeGraph, file_path: &Path, line: u32) -> Option<SymbolId> {
    graph
        .all_nodes()
        .filter(|n| {
            matches!(n.kind, NodeKind::Function | NodeKind::Method | NodeKind::Constructor)
                && n.file_path == file_path
                && n.span.start_line <= line
                && n.span.end_line >= line
        })
        .min_by_key(|n| n.span.end_line - n.span.start_line)
        .map(|n| n.id)
}

/// Convert a file:// URI to a PathBuf.
fn uri_to_path(uri: &str) -> PathBuf {
    uri.strip_prefix("file://")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Find the column position of a name on a given line, using a file cache.
fn find_name_column(
    file_cache: &mut HashMap<PathBuf, Vec<String>>,
    file_path: &Path,
    line: u32,
    name: &str,
) -> u32 {
    file_cache
        .entry(file_path.to_path_buf())
        .or_insert_with(|| {
            std::fs::read_to_string(file_path)
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        })
        .get(line as usize)
        .and_then(|line_text| line_text.find(name))
        .unwrap_or(0) as u32
}

fn count_functions(graph: &CodeGraph, languages: &[Language]) -> usize {
    graph
        .all_nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Constructor
            ) && languages.contains(&n.language)
                && !graph.is_phantom(n.id)
        })
        .count()
}

fn is_on_path(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, GirEdge, Span};
    use std::path::PathBuf;

    // ── language_to_lsp_id ─────────────────────────────────

    #[test]
    fn language_to_lsp_id_all_variants() {
        assert_eq!(language_to_lsp_id(Language::Python), "python");
        assert_eq!(language_to_lsp_id(Language::TypeScript), "typescript");
        assert_eq!(language_to_lsp_id(Language::JavaScript), "javascript");
        assert_eq!(language_to_lsp_id(Language::Rust), "rust");
        assert_eq!(language_to_lsp_id(Language::Go), "go");
        assert_eq!(language_to_lsp_id(Language::Java), "java");
        assert_eq!(language_to_lsp_id(Language::Cpp), "cpp");
        assert_eq!(language_to_lsp_id(Language::C), "c");
        assert_eq!(language_to_lsp_id(Language::CSharp), "csharp");
        assert_eq!(language_to_lsp_id(Language::Ruby), "ruby");
        assert_eq!(language_to_lsp_id(Language::Kotlin), "kotlin");
        assert_eq!(language_to_lsp_id(Language::Php), "php");
        assert_eq!(language_to_lsp_id(Language::Svelte), "svelte");
    }

    // ── uri_to_path ────────────────────────────────────────

    #[test]
    fn uri_to_path_normal() {
        let path = uri_to_path("file:///home/user/project/main.py");
        assert_eq!(path, PathBuf::from("/home/user/project/main.py"));
    }

    #[test]
    fn uri_to_path_no_prefix() {
        let path = uri_to_path("/just/a/path");
        assert_eq!(path, PathBuf::from(""));
    }

    #[test]
    fn uri_to_path_empty() {
        let path = uri_to_path("");
        assert_eq!(path, PathBuf::from(""));
    }

    #[test]
    fn uri_to_path_with_spaces() {
        let path = uri_to_path("file:///home/user/my project/main.py");
        assert_eq!(path, PathBuf::from("/home/user/my project/main.py"));
    }

    // ── find_containing_function ───────────────────────────

    fn make_fn_node(name: &str, file: &str, start: u32, end: u32) -> GirNode {
        GirNode::new(
            name.into(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(start, 0, end, 0),
            Language::Python,
        )
    }

    fn make_file_node(file: &str) -> GirNode {
        GirNode::new(
            file.into(),
            NodeKind::File,
            PathBuf::from(file),
            Span::new(0, 0, 0, 0),
            Language::Python,
        )
    }

    #[test]
    fn find_containing_function_empty_graph() {
        let graph = CodeGraph::new();
        let result = find_containing_function(&graph, Path::new("test.py"), 10);
        assert!(result.is_none());
    }

    #[test]
    fn find_containing_function_exact_match() {
        let mut graph = CodeGraph::new();
        let file = make_file_node("test.py");
        let file_id = file.id;
        graph.add_node(file);
        let func = make_fn_node("my_func", "test.py", 5, 15);
        let func_id = func.id;
        graph.add_node(func);
        graph.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));

        let result = find_containing_function(&graph, Path::new("test.py"), 10);
        assert_eq!(result, Some(func_id));
    }

    #[test]
    fn find_containing_function_narrowest_wins() {
        let mut graph = CodeGraph::new();
        let file = make_file_node("test.py");
        let file_id = file.id;
        graph.add_node(file);

        // Outer function: lines 1-50
        let outer = make_fn_node("outer", "test.py", 1, 50);
        let outer_id = outer.id;
        graph.add_node(outer);
        graph.add_edge(file_id, outer_id, GirEdge::new(EdgeKind::Contains));

        // Inner function: lines 10-20 (narrower)
        let inner = make_fn_node("inner", "test.py", 10, 20);
        let inner_id = inner.id;
        graph.add_node(inner);
        graph.add_edge(file_id, inner_id, GirEdge::new(EdgeKind::Contains));

        let result = find_containing_function(&graph, Path::new("test.py"), 15);
        assert_eq!(result, Some(inner_id));
    }

    #[test]
    fn find_containing_function_wrong_file() {
        let mut graph = CodeGraph::new();
        let file = make_file_node("a.py");
        let file_id = file.id;
        graph.add_node(file);
        let func = make_fn_node("my_func", "a.py", 1, 10);
        let func_id = func.id;
        graph.add_node(func);
        graph.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));

        let result = find_containing_function(&graph, Path::new("b.py"), 5);
        assert!(result.is_none());
    }

    // ── count_functions ────────────────────────────────────

    #[test]
    fn count_functions_empty_graph() {
        let graph = CodeGraph::new();
        assert_eq!(count_functions(&graph, &[Language::Python]), 0);
    }

    #[test]
    fn count_functions_filters_by_language() {
        let mut graph = CodeGraph::new();
        let file = make_file_node("test.py");
        let file_id = file.id;
        graph.add_node(file);

        let py_fn = GirNode::new(
            "py_func".into(), NodeKind::Function,
            PathBuf::from("test.py"), Span::new(1, 0, 5, 0), Language::Python,
        );
        let py_id = py_fn.id;
        graph.add_node(py_fn);
        graph.add_edge(file_id, py_id, GirEdge::new(EdgeKind::Contains));

        let rs_fn = GirNode::new(
            "rs_func".into(), NodeKind::Function,
            PathBuf::from("test.rs"), Span::new(1, 0, 5, 0), Language::Rust,
        );
        let rs_id = rs_fn.id;
        graph.add_node(rs_fn);
        graph.add_edge(file_id, rs_id, GirEdge::new(EdgeKind::Contains));

        assert_eq!(count_functions(&graph, &[Language::Python]), 1);
        assert_eq!(count_functions(&graph, &[Language::Rust]), 1);
        assert_eq!(count_functions(&graph, &[Language::Python, Language::Rust]), 2);
        assert_eq!(count_functions(&graph, &[Language::Go]), 0);
    }

    // ── LspEnhanceResult ───────────────────────────────────

    #[test]
    fn lsp_enhance_result_default() {
        let result = LspEnhanceResult::default();
        assert!(result.servers_used.is_empty());
        assert_eq!(result.edges_added, 0);
        assert_eq!(result.functions_queried, 0);
    }

    // ── LSP_SERVERS ────────────────────────────────────────

    #[test]
    fn lsp_servers_table_nonempty() {
        assert!(!LSP_SERVERS.is_empty());
        for (name, langs) in LSP_SERVERS {
            assert!(!name.is_empty());
            assert!(!langs.is_empty());
        }
    }

    // ── process_incoming_call ──────────────────────────────

    #[test]
    fn process_incoming_call_missing_caller() {
        let mut graph = CodeGraph::new();
        let file = make_file_node("test.py");
        let file_id = file.id;
        graph.add_node(file);
        let func = make_fn_node("target", "test.py", 1, 5);
        let func_id = func.id;
        graph.add_node(func);
        graph.add_edge(file_id, func_id, GirEdge::new(EdgeKind::Contains));

        let call = serde_json::json!({
            "from": {
                "name": "nonexistent",
                "uri": "file:///test.py",
                "range": { "start": { "line": 99, "character": 0 }, "end": { "line": 99, "character": 10 } }
            }
        });

        let added = process_incoming_call(&mut graph, &call, func_id, "target", Path::new("test.py"));
        assert_eq!(added, 0);
    }

    // ── enhance_with_lsp (empty graph) ─────────────────────

    #[test]
    fn enhance_with_lsp_empty_graph() {
        let mut graph = CodeGraph::new();
        let result = enhance_with_lsp(&mut graph, Path::new("/tmp"));
        assert!(result.servers_used.is_empty());
        assert_eq!(result.edges_added, 0);
        assert_eq!(result.functions_queried, 0);
    }
}
