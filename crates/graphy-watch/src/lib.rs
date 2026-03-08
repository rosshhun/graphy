use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use graphy_core::ParseOutput;
use notify_debouncer_full::{
    new_debouncer,
    notify::{RecursiveMode, Watcher},
    DebounceEventResult,
};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use graphy_core::{CodeGraph, Language};
use graphy_search::SearchIndex;
use sha2::{Digest, Sha256};

/// Compute a SHA-256 hash of the given source content, returned as a hex string.
fn hash_source(source: &str) -> String {
    let hash = Sha256::digest(source.as_bytes());
    format!("{hash:x}")
}

/// Result of reading and parsing a single file outside the graph lock.
enum FileAction {
    /// File was deleted — remove from graph.
    Deleted(PathBuf),
    /// File parsed successfully — remove old nodes, merge new output, update hash.
    Parsed {
        path: PathBuf,
        output: ParseOutput,
        new_hash: String,
    },
    /// File unchanged or failed to read/parse — skip.
    Skip,
}

/// Callback invoked after each successful re-index.
pub type OnReindexCallback = Box<dyn Fn(usize, usize, usize) + Send + Sync>;

/// File watcher that triggers incremental re-indexing on changes.
pub struct FileWatcher {
    root: PathBuf,
    graph: Arc<RwLock<CodeGraph>>,
    search: Option<Arc<SearchIndex>>,
    debounce_ms: u64,
    reindex_count: AtomicU64,
    /// Cached content hashes (path -> SHA-256 hex of last-parsed source).
    old_source_hashes: Mutex<HashMap<PathBuf, String>>,
    /// Warm LSP client for incremental queries (persists across re-indexes).
    lsp_client: Mutex<Option<graphy_analysis::lsp_enhance::LspClient>>,
    /// Whether to use LSP enhancement.
    use_lsp: bool,
    /// Optional callback after re-index (files_changed, node_count, edge_count).
    on_reindex: Option<OnReindexCallback>,
}

impl FileWatcher {
    pub fn new(root: PathBuf, graph: Arc<RwLock<CodeGraph>>) -> Self {
        Self {
            root,
            graph,
            search: None,
            debounce_ms: 200,
            reindex_count: AtomicU64::new(0),
            old_source_hashes: Mutex::new(HashMap::new()),
            lsp_client: Mutex::new(None),
            use_lsp: false,
            on_reindex: None,
        }
    }

    /// Set the search index to keep in sync with graph changes.
    pub fn with_search(mut self, search: Arc<SearchIndex>) -> Self {
        self.search = Some(search);
        self
    }

    /// Enable LSP enhancement for incremental re-indexing.
    pub fn with_lsp(mut self, use_lsp: bool) -> Self {
        self.use_lsp = use_lsp;
        self
    }

    /// Set a callback invoked after each re-index (files_changed, node_count, edge_count).
    pub fn with_on_reindex(mut self, cb: OnReindexCallback) -> Self {
        self.on_reindex = Some(cb);
        self
    }

    /// Start watching for file changes. This runs until the returned handle is dropped.
    pub async fn watch(&self) -> Result<()> {
        let root = self.root.clone();
        let debounce_ms = self.debounce_ms;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<PathBuf>>(100);

        // Spawn the notify watcher in a blocking thread
        let (init_tx, init_rx) = tokio::sync::oneshot::channel::<Result<()>>();

        let _watcher_handle = tokio::task::spawn_blocking(move || {
            let tx_clone = tx.clone();
            let root_clone = root.clone();

            let debouncer = new_debouncer(
                Duration::from_millis(debounce_ms),
                None,
                move |result: DebounceEventResult| {
                    match result {
                        Ok(events) => {
                            let changed_files: Vec<PathBuf> = events
                                .iter()
                                .flat_map(|e| e.paths.iter().cloned())
                                .filter(|p| is_supported_file(p))
                                .collect();

                            if !changed_files.is_empty() {
                                let _ = tx_clone.blocking_send(changed_files);
                            }
                        }
                        Err(errors) => {
                            for e in errors {
                                error!("Watch error: {e}");
                            }
                        }
                    }
                },
            );

            let mut debouncer = match debouncer {
                Ok(d) => d,
                Err(e) => {
                    let _ = init_tx
                        .send(Err(anyhow::anyhow!("Failed to create file watcher: {e}")));
                    return;
                }
            };

            if let Err(e) = debouncer
                .watcher()
                .watch(&root_clone, RecursiveMode::Recursive)
            {
                let _ = init_tx.send(Err(anyhow::anyhow!(
                    "Failed to watch directory {}: {e}",
                    root_clone.display()
                )));
                return;
            }

            info!("File watcher active on {}", root_clone.display());
            let _ = init_tx.send(Ok(()));

            // Keep the debouncer alive
            std::thread::park();
        });

        // Wait for watcher initialization — propagate errors instead of panicking
        match init_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(anyhow::anyhow!("Watcher thread exited unexpectedly")),
        }

        // Process file change events
        while let Some(changed_files) = rx.recv().await {
            let unique: Vec<_> = {
                let mut seen = std::collections::HashSet::new();
                changed_files
                    .into_iter()
                    .filter(|p| seen.insert(p.clone()))
                    .collect()
            };

            info!("{} file(s) changed, re-indexing...", unique.len());

            for path in &unique {
                debug!("  Changed: {}", path.display());
            }

            if let Err(e) = self.reindex_files(&unique).await {
                warn!("Re-indexing failed: {e}");
            }
        }

        Ok(())
    }

    async fn reindex_files(&self, files: &[PathBuf]) -> Result<()> {
        // ── Phase 1: Read files, hash, and parse OUTSIDE the graph lock ──
        // Uses parse_files() with rayon for parallel parsing when multiple files changed.
        let actions = {
            let hashes = self.old_source_hashes.lock().await;
            let mut to_parse: Vec<(PathBuf, String, String)> = Vec::new();
            let mut actions = Vec::with_capacity(files.len());
            let mut action_indices: Vec<usize> = Vec::new(); // maps to_parse index -> actions index

            for path in files {
                if !path.exists() {
                    info!("File deleted: {}", path.display());
                    actions.push(FileAction::Deleted(path.clone()));
                    continue;
                }

                let source = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to read {}: {e}", path.display());
                        actions.push(FileAction::Skip);
                        continue;
                    }
                };

                let new_hash = hash_source(&source);
                if let Some(old_hash) = hashes.get(path) {
                    if *old_hash == new_hash {
                        debug!("Skipping {} (content unchanged)", path.display());
                        actions.push(FileAction::Skip);
                        continue;
                    }
                }

                // Queue for parsing
                let idx = actions.len();
                actions.push(FileAction::Skip); // placeholder
                action_indices.push(idx);
                to_parse.push((path.clone(), source, new_hash));
            }

            // Drop hash lock before parsing
            drop(hashes);

            // Parse files: use rayon parallel parsing for multiple files,
            // direct single-file parse to avoid rayon overhead for 1 file.
            if to_parse.len() == 1 {
                let (path, source, new_hash) = &to_parse[0];
                match graphy_parser::parse_file(path, source) {
                    Ok(output) => {
                        actions[action_indices[0]] = FileAction::Parsed {
                            path: path.clone(),
                            output,
                            new_hash: new_hash.clone(),
                        };
                    }
                    Err(e) => {
                        warn!("Failed to parse {}: {e} (keeping existing data)", path.display());
                    }
                }
            } else if !to_parse.is_empty() {
                let file_contents: Vec<(PathBuf, String)> = to_parse
                    .iter()
                    .map(|(p, s, _)| (p.clone(), s.clone()))
                    .collect();

                // parse_files uses rayon internally for parallel parsing
                let results = tokio::task::block_in_place(|| {
                    graphy_parser::parse_files(&file_contents)
                });

                for (i, (path, result)) in results.into_iter().enumerate() {
                    match result {
                        Ok(output) => {
                            actions[action_indices[i]] = FileAction::Parsed {
                                path,
                                output,
                                new_hash: to_parse[i].2.clone(),
                            };
                        }
                        Err(e) => {
                            warn!("Failed to parse {}: {e} (keeping existing data)", path.display());
                        }
                    }
                }
            }

            actions
        };

        // Pre-load source files for complexity computation OUTSIDE the lock.
        // This avoids disk I/O while holding the graph write lock.
        let mut complexity_cache: std::collections::HashMap<PathBuf, Vec<String>> =
            std::collections::HashMap::new();
        for action in &actions {
            if let FileAction::Parsed { path, .. } = action {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let lines: Vec<String> = content.lines().map(String::from).collect();
                    complexity_cache.insert(path.clone(), lines);
                }
            }
        }

        // ── Phase 2: Apply mutations under the graph write lock ──
        let any_changed = {
            let mut graph = self.graph.write().await;
            let mut hashes = self.old_source_hashes.lock().await;
            let mut any_changed = false;

            // Track which files actually changed for scoped re-analysis.
            let mut changed_paths: Vec<PathBuf> = Vec::new();

            for action in actions {
                match action {
                    FileAction::Deleted(path) => {
                        graph.remove_file(&path);
                        hashes.remove(&path);
                        changed_paths.push(path);
                        any_changed = true;
                    }
                    FileAction::Parsed {
                        path,
                        output,
                        new_hash,
                    } => {
                        let node_count = output.nodes.len();
                        // remove_file cleans up the old nodes AND their incident edges
                        // (including Imports/ImportsFrom/Calls) — no need to nuke all edges.
                        graph.remove_file(&path);
                        graph.merge(output);
                        hashes.insert(path.clone(), new_hash);
                        any_changed = true;
                        debug!("Re-indexed {}: {} nodes", path.display(), node_count);
                        changed_paths.push(path);
                    }
                    FileAction::Skip => {}
                }
            }

            // Re-run import resolution and call tracing on the whole graph.
            // We don't delete all edges first — remove_file already cleaned up
            // the changed files' edges, so resolve_imports/resolve_calls only
            // ADD edges for the re-parsed nodes. Existing edges for unchanged
            // files are preserved.
            if any_changed {
                graphy_analysis::import_resolution::resolve_imports(&mut graph, &self.root);
                graphy_analysis::call_tracing::resolve_calls(&mut graph, &self.root);
                // Compute complexity using pre-loaded source (no disk I/O under lock).
                graphy_analysis::complexity::compute_complexity_with_cache(
                    &mut graph,
                    Some(&changed_paths),
                    Some(complexity_cache.clone()),
                );
                // Re-run dead code detection so liveness scores stay fresh.
                graphy_analysis::dead_code::detect_dead_code(&mut graph);
                debug!("Ran incremental import resolution, call tracing, complexity, and dead code");
            }

            // Notify warm LSP client of file changes (reuse parsed source to avoid re-reading).
            if self.use_lsp && any_changed {
                let mut lsp = self.lsp_client.lock().await;
                if let Some(client) = lsp.as_mut() {
                    if client.is_alive() {
                        for path in &changed_paths {
                            if path.exists() {
                                if let Ok(content) = std::fs::read_to_string(path) {
                                    let _ = client.did_change(path, &content);
                                }
                            }
                        }
                    } else {
                        // Server crashed — drop it, lazy restart on next change
                        *lsp = None;
                        debug!("LSP server crashed, will restart on next change");
                    }
                } else {
                    // Lazy start: create LSP client on first change
                    match graphy_analysis::lsp_enhance::LspClient::start("rust-analyzer", &self.root)
                        .or_else(|_| graphy_analysis::lsp_enhance::LspClient::start("pyright-langserver", &self.root))
                        .or_else(|_| graphy_analysis::lsp_enhance::LspClient::start("typescript-language-server", &self.root))
                    {
                        Ok(client) => {
                            info!("Warm LSP client started for watch mode");
                            *lsp = Some(client);
                        }
                        Err(_) => {
                            debug!("No LSP server available for warm mode");
                        }
                    }
                }
            }

            let count = self.reindex_count.fetch_add(1, Ordering::Relaxed) + 1;
            let (nc, ec) = (graph.node_count(), graph.edge_count());
            info!(
                "Graph updated: {} nodes, {} edges (reindex #{})",
                nc, ec, count,
            );

            // Notify MCP server of graph update
            if any_changed {
                if let Some(cb) = &self.on_reindex {
                    cb(files.len(), nc, ec);
                }
            }

            any_changed
        };

        // ── Phase 3: Incrementally update search index for changed files ──
        if any_changed {
            if let Some(search) = &self.search {
                let graph = self.graph.read().await;
                if let Err(e) = search.update_files(&graph, files) {
                    warn!("Failed to update search index: {e}");
                }
            }
        }

        Ok(())
    }
}

fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(Language::from_extension)
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ── hash_source ────────────────────────────────────────

    #[test]
    fn hash_source_deterministic() {
        let h1 = hash_source("fn main() {}");
        let h2 = hash_source("fn main() {}");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_source_different_content() {
        let h1 = hash_source("fn main() {}");
        let h2 = hash_source("fn main() { println!(); }");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_source_empty_string() {
        let h = hash_source("");
        assert!(!h.is_empty());
        // SHA-256 of empty string is a known constant
        assert_eq!(h, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn hash_source_unicode() {
        let h1 = hash_source("// 日本語コメント");
        let h2 = hash_source("// 日本語コメント");
        assert_eq!(h1, h2);
    }

    // ── is_supported_file ──────────────────────────────────

    #[test]
    fn is_supported_file_python() {
        assert!(is_supported_file(Path::new("app.py")));
    }

    #[test]
    fn is_supported_file_typescript() {
        assert!(is_supported_file(Path::new("index.ts")));
        assert!(is_supported_file(Path::new("index.tsx")));
    }

    #[test]
    fn is_supported_file_rust() {
        assert!(is_supported_file(Path::new("main.rs")));
    }

    #[test]
    fn is_supported_file_javascript() {
        assert!(is_supported_file(Path::new("app.js")));
        assert!(is_supported_file(Path::new("app.jsx")));
    }

    #[test]
    fn is_supported_file_unsupported() {
        assert!(!is_supported_file(Path::new("readme.md")));
        assert!(!is_supported_file(Path::new("image.png")));
        assert!(!is_supported_file(Path::new("data.json")));
    }

    #[test]
    fn is_supported_file_no_extension() {
        assert!(!is_supported_file(Path::new("Makefile")));
        assert!(!is_supported_file(Path::new(".")));
    }

    #[test]
    fn is_supported_file_svelte() {
        assert!(is_supported_file(Path::new("Component.svelte")));
    }

    // ── FileWatcher construction ───────────────────────────

    #[test]
    fn file_watcher_new() {
        let graph = Arc::new(RwLock::new(CodeGraph::new()));
        let watcher = FileWatcher::new(PathBuf::from("/tmp/test"), graph.clone());
        assert_eq!(watcher.root, PathBuf::from("/tmp/test"));
        assert!(!watcher.use_lsp);
        assert!(watcher.search.is_none());
        assert!(watcher.on_reindex.is_none());
    }

    #[test]
    fn file_watcher_with_lsp() {
        let graph = Arc::new(RwLock::new(CodeGraph::new()));
        let watcher = FileWatcher::new(PathBuf::from("/tmp"), graph).with_lsp(true);
        assert!(watcher.use_lsp);
    }

    #[test]
    fn file_watcher_with_lsp_false() {
        let graph = Arc::new(RwLock::new(CodeGraph::new()));
        let watcher = FileWatcher::new(PathBuf::from("/tmp"), graph).with_lsp(false);
        assert!(!watcher.use_lsp);
    }

    // ── FileAction enum ────────────────────────────────────

    #[test]
    fn file_action_variants() {
        // Just verify the enum constructors work
        let _deleted = FileAction::Deleted(PathBuf::from("test.py"));
        let _skip = FileAction::Skip;
        let _parsed = FileAction::Parsed {
            path: PathBuf::from("test.py"),
            output: ParseOutput::default(),
            new_hash: "abc123".into(),
        };
    }
}
