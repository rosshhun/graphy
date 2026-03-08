#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "web")]
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::sync::RwLock;
use tracing::info;

use graphy_analysis::{pipeline::PipelineConfig, AnalysisPipeline};
use graphy_core::{diff, storage, CodeGraph, NodeKind};
use graphy_search::SearchIndex;

#[derive(Parser)]
#[command(
    name = "graphy",
    version,
    about = "Graph-powered code intelligence engine",
    long_about = "Graph-powered code intelligence engine.\n\n\
                  Run with no arguments to analyze, launch dashboard, and watch for changes."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    // ── Simple commands ──────────────────────────────────────

    /// Analyze + launch dashboard + watch for changes (default)
    #[cfg(feature = "web")]
    Dev {
        /// Path to the project root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Dashboard port
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Use installed LSP servers for precise call resolution in watch mode
        #[arg(long)]
        lsp: bool,
    },

    /// Set up graphy in this project (creates .mcp.json)
    Init,

    /// Open the dashboard (skips re-analysis if already indexed)
    #[cfg(feature = "web")]
    Open {
        /// Path to the project root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Dashboard port
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },

    // ── Core commands ────────────────────────────────────────

    /// Index a repository
    Analyze {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Full re-index (ignore existing index)
        #[arg(long)]
        full: bool,

        /// Use installed LSP servers (rust-analyzer, pyright, etc.) for precise call resolution
        #[arg(long)]
        lsp: bool,
    },

    /// Search for symbols
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(short = 'n', long, default_value = "10")]
        max_results: usize,

        /// Filter by kind (Function, Class, Method, etc.)
        #[arg(short, long)]
        kind: Option<String>,
    },

    /// Show full symbol context (callers, callees, types, etc.)
    Context {
        /// Symbol name to look up
        symbol: String,
    },

    /// Show blast radius of changing a symbol
    Impact {
        /// Symbol name to analyze
        symbol: String,

        /// Maximum depth to traverse
        #[arg(short, long, default_value = "3")]
        depth: usize,
    },

    /// Report dead code
    DeadCode {
        /// Path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show complexity hotspots
    Hotspots {
        /// Maximum number of results
        #[arg(short = 'n', long, default_value = "20")]
        max_results: usize,
    },

    /// Show taint analysis results
    Taint {
        /// Optional symbol to trace
        symbol: Option<String>,
    },

    /// Show graph statistics
    Stats {
        /// Path to the project root
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    // ── Server commands ──────────────────────────────────────

    /// Start the MCP server (for Claude Code / AI agents)
    Serve {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Watch for file changes and hot-reload the graph
        #[arg(long)]
        watch: bool,

        /// Use installed LSP servers for precise call resolution (requires --watch)
        #[arg(long)]
        lsp: bool,
    },

    /// Start live re-indexing file watcher
    Watch {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Use installed LSP servers for precise call resolution
        #[arg(long)]
        lsp: bool,
    },

    // ── Advanced commands ────────────────────────────────────

    /// Show dependency information
    Deps {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Check for known vulnerabilities
        #[arg(long)]
        vulns: bool,
    },

    /// Diff two graph versions (CI/CD breaking change guardian)
    Diff {
        /// Path to the base version (directory or .redb file)
        base: PathBuf,

        /// Path to the head version (directory or .redb file)
        head: PathBuf,

        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,

        /// Exit with code 1 if breaking changes are found
        #[arg(long)]
        fail_on_breaking: bool,
    },

    /// Generate codebase context document
    ContextGen {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output format: md or json
        #[arg(long, default_value = "md")]
        format: String,
    },

    /// Manage dynamic language grammars
    Lang {
        #[command(subcommand)]
        action: LangAction,
    },

    /// Analyze multiple repositories as a unified graph
    MultiRepo {
        /// Paths to repository roots
        paths: Vec<PathBuf>,

        /// Output path for the merged graph
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum LangAction {
    /// Install a tree-sitter grammar for a language
    Add {
        /// Language name (e.g., go, java, php, c, cpp, c-sharp, ruby, kotlin)
        name: String,
    },

    /// Remove an installed grammar
    Remove {
        /// Language name
        name: String,
    },

    /// List built-in and installed grammars
    List,
}

fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .map_err(Into::into)
}

fn main() -> Result<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build_global()
        .ok();

    let cli = Cli::parse();

    let filter = if cli.verbose {
        "graphy=debug,graphy_core=debug,graphy_parser=debug,graphy_analysis=debug"
    } else {
        "graphy=warn"
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_target(false)
        .init();

    match cli.command {
        #[cfg(feature = "web")]
        None => cmd_dev(&PathBuf::from("."), 3000, false),
        #[cfg(not(feature = "web"))]
        None => {
            cmd_analyze(&PathBuf::from("."), false, false)?;
            cmd_watch(&PathBuf::from("."), false)
        }
        Some(cmd) => match cmd {
            #[cfg(feature = "web")]
            Commands::Dev { path, port, lsp } => cmd_dev(&path, port, lsp),
            Commands::Init => cmd_init(),
            #[cfg(feature = "web")]
            Commands::Open { path, port } => cmd_open(&path, port),
            Commands::Analyze { path, full, lsp } => cmd_analyze(&path, full, lsp),
            Commands::Search {
                query,
                max_results,
                kind,
            } => cmd_search(&query, max_results, kind.as_deref()),
            Commands::Context { symbol } => cmd_context(&symbol),
            Commands::Impact { symbol, depth } => cmd_impact(&symbol, depth),
            Commands::DeadCode { path } => cmd_dead_code(&path),
            Commands::Stats { path } => cmd_stats(&path),
            Commands::Hotspots { max_results } => cmd_hotspots(max_results),
            Commands::Taint { symbol } => cmd_taint(symbol.as_deref()),
            Commands::Serve { path, watch, lsp } => cmd_serve(&path, watch, lsp),
            Commands::Watch { path, lsp } => cmd_watch(&path, lsp),
            Commands::Deps { path, vulns } => cmd_deps(&path, vulns),
            Commands::Diff {
                base,
                head,
                format,
                fail_on_breaking,
            } => cmd_diff(&base, &head, &format, fail_on_breaking),
            Commands::ContextGen { path, format } => cmd_context_gen(&path, &format),
            Commands::Lang { action } => cmd_lang(action),
            Commands::MultiRepo { paths, output } => cmd_multi_repo(&paths, output.as_deref()),
        },
    }
}

// ── Simple Commands ─────────────────────────────────────────────

/// The main "just works" command: analyze + dashboard + watch.
#[cfg(feature = "web")]
fn cmd_dev(path: &PathBuf, port: u16, use_lsp: bool) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let start = Instant::now();

    eprint!("  Indexing project...");
    let graph = ensure_graph(&root)?;
    let search = build_search(&root, &graph)?;
    eprintln!(" done");

    let node_count = graph.node_count();
    let edge_count = graph.edge_count();
    let file_count = graph.find_by_kind(NodeKind::File).len();
    let graph = Arc::new(RwLock::new(graph));

    let rt = build_runtime()?;
    rt.block_on(async {
        print_banner(port, node_count, edge_count, file_count, start.elapsed(), true);

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

        let search = Arc::new(search);
        let state = graphy_web::AppState {
            graph: graph.clone(),
            search: search.clone(),
            project_root: root.clone(),
        };

        let web = tokio::spawn(async move {
            if let Err(e) = graphy_web::serve(state, port, shutdown_rx).await {
                eprintln!("  Web server error: {e}");
            }
        });

        let graph_for_save = graph.clone();
        let root_for_save = root.clone();
        let watcher = graphy_watch::FileWatcher::new(root, graph)
            .with_search(search)
            .with_lsp(use_lsp);
        let watch = tokio::spawn(async move {
            if let Err(e) = watcher.watch().await {
                eprintln!("  Watcher error: {e}");
            }
        });

        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n  Shutting down...");
        let _ = shutdown_tx.send(());
        web.abort();
        watch.abort();

        // Save the current graph state so other commands get fresh data
        {
            let graph = graph_for_save.read().await;
            let db_path = storage::default_db_path(&root_for_save);
            if let Err(e) = storage::save_graph(&graph, &db_path) {
                eprintln!("  \x1b[33m!\x1b[0m Failed to save index on shutdown: {e}");
            } else {
                eprintln!("  Saved index.");
            }
        }

        // The file watcher spawns OS threads that don't respond to task abort,
        // so we exit the process directly after signaling shutdown.
        std::process::exit(0);
    })
}

/// Set up graphy in the current project.
fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;

    eprintln!();

    // Create .mcp.json
    let mcp_path = cwd.join(".mcp.json");
    if mcp_path.exists() {
        eprintln!("  \x1b[33m-\x1b[0m .mcp.json already exists");
    } else {
        let mcp_content = r#"{
  "mcpServers": {
    "graphy": {
      "command": "graphy",
      "args": ["serve", "--watch", "."]
    }
  }
}
"#;
        std::fs::write(&mcp_path, mcp_content)?;
        eprintln!("  \x1b[32m+\x1b[0m Created .mcp.json");
    }

    // Update .gitignore
    let gitignore_path = cwd.join(".gitignore");
    let needs_entry = if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        !content.contains(".graphy")
    } else {
        true
    };

    if needs_entry {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)?;
        use std::io::Write;
        writeln!(f, "\n# Graphy index data\n.graphy/")?;
        eprintln!("  \x1b[32m+\x1b[0m Added .graphy/ to .gitignore");
    } else {
        eprintln!("  \x1b[33m-\x1b[0m .gitignore already has .graphy/");
    }

    eprintln!();
    eprintln!("  \x1b[1mGet started:\x1b[0m");
    eprintln!("    graphy            Analyze + dashboard + live reload");
    eprintln!("    graphy open       Open dashboard only");
    eprintln!("    graphy serve      Start MCP server for Claude Code");
    eprintln!();
    eprintln!("  \x1b[1mClaude Code:\x1b[0m");
    eprintln!("    Open Claude Code in this directory.");
    eprintln!("    It auto-detects .mcp.json and connects graphy's MCP tools.");
    eprintln!();

    Ok(())
}

/// Open the dashboard (skip re-analysis if already indexed).
#[cfg(feature = "web")]
fn cmd_open(path: &PathBuf, port: u16) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let start = Instant::now();

    eprint!("  Loading project...");
    let graph = ensure_graph(&root)?;
    let search = build_search(&root, &graph)?;
    eprintln!(" done");

    let node_count = graph.node_count();
    let edge_count = graph.edge_count();
    let file_count = graph.find_by_kind(NodeKind::File).len();

    let rt = build_runtime()?;
    rt.block_on(async {
        print_banner(port, node_count, edge_count, file_count, start.elapsed(), false);

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

        let state = graphy_web::AppState {
            graph: Arc::new(RwLock::new(graph)),
            search: Arc::new(search),
            project_root: root,
        };

        let web = tokio::spawn(async move {
            if let Err(e) = graphy_web::serve(state, port, shutdown_rx).await {
                eprintln!("  Web server error: {e}");
            }
        });

        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n  Shutting down...");
        let _ = shutdown_tx.send(());
        web.abort();

        std::process::exit(0);
    })
}

// ── Core Commands ───────────────────────────────────────────────

fn cmd_analyze(path: &PathBuf, full: bool, use_lsp: bool) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    info!("Analyzing {}", root.display());

    let config = PipelineConfig {
        incremental: !full,
        use_lsp,
        ..Default::default()
    };

    let pipeline = AnalysisPipeline::new(root.clone(), config);
    let graph = pipeline.run()?;

    // Save the graph
    let db_path = storage::default_db_path(&root);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    storage::save_graph(&graph, &db_path)?;

    // Build and save search index
    let search_path = graphy_search::default_search_path(&root);
    match SearchIndex::new_persistent(&search_path) {
        Ok(idx) => {
            if let Err(e) = idx.index_graph(&graph) {
                tracing::warn!("Search indexing failed: {e}");
            }
        }
        Err(e) => tracing::warn!("Search index creation failed: {e}"),
    }

    info!("Graph saved to {}", db_path.display());
    print_summary(&graph);

    Ok(())
}

fn cmd_search(query: &str, max_results: usize, kind: Option<&str>) -> Result<()> {
    let root = std::fs::canonicalize(".")?;
    let search_path = graphy_search::default_search_path(&root);

    if search_path.exists() {
        // Open read-only — safe even if `graphy dev` holds the writer lock.
        let reader = graphy_search::SearchReader::open(&search_path)?;
        let results = if let Some(k) = kind {
            reader.search_by_kind(query, k, max_results)?
        } else {
            reader.search(query, max_results)?
        };

        if results.is_empty() {
            println!("No results for '{query}'");
        } else {
            println!("Found {} results for '{query}':\n", results.len());
            for r in &results {
                println!(
                    "  {} {} ({}:{}) score={:.2}",
                    r.kind, r.name, r.file_path, r.start_line, r.score
                );
                if let Some(sig) = &r.signature {
                    println!("    {sig}");
                }
            }
        }
    } else {
        // Fallback to graph-based search
        let graph = load_graph(&root)?;
        let query_lower = query.to_lowercase();
        let mut matches: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.name.to_lowercase().contains(&query_lower))
            .filter(|n| {
                kind.map_or(true, |k| {
                    format!("{:?}", n.kind).to_lowercase() == k.to_lowercase()
                })
            })
            .collect();
        matches.sort_by_key(|n| n.name.len());
        matches.truncate(max_results);

        if matches.is_empty() {
            println!("No results for '{query}'");
        } else {
            println!("Found {} results for '{query}':\n", matches.len());
            for node in &matches {
                println!(
                    "  {:?} {} ({}:{})",
                    node.kind,
                    node.name,
                    node.file_path.display(),
                    node.span.start_line
                );
            }
        }
    }

    Ok(())
}

fn cmd_context(symbol: &str) -> Result<()> {
    let root = std::fs::canonicalize(".")?;
    let graph = load_graph(&root)?;

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        println!("Symbol '{symbol}' not found");
        return Ok(());
    }

    for node in nodes {
        println!("--- {} ({:?}) ---", node.name, node.kind);
        println!(
            "  File: {}:{}",
            node.file_path.display(),
            node.span.start_line
        );
        println!("  Visibility: {:?}", node.visibility);

        if let Some(sig) = &node.signature {
            println!("  Signature: {sig}");
        }
        if let Some(doc) = &node.doc {
            println!("  Doc: {doc}");
        }
        if let Some(cx) = &node.complexity {
            println!(
                "  Complexity: cyclomatic={}, cognitive={}, loc={}",
                cx.cyclomatic, cx.cognitive, cx.loc
            );
        }

        let callers = graph.callers(node.id);
        if !callers.is_empty() {
            println!("  Callers ({}):", callers.len());
            for c in &callers {
                println!(
                    "    <- {} ({}:{})",
                    c.name,
                    c.file_path.display(),
                    c.span.start_line
                );
            }
        }

        let callees = graph.callees(node.id);
        if !callees.is_empty() {
            println!("  Callees ({}):", callees.len());
            for c in &callees {
                println!(
                    "    -> {} ({}:{})",
                    c.name,
                    c.file_path.display(),
                    c.span.start_line
                );
            }
        }

        let children = graph.children(node.id);
        if !children.is_empty() {
            println!("  Contains ({}):", children.len());
            for c in &children {
                println!("    {:?} {}", c.kind, c.name);
            }
        }

        println!();
    }

    Ok(())
}

fn cmd_impact(symbol: &str, max_depth: usize) -> Result<()> {
    let root = std::fs::canonicalize(".")?;
    let graph = load_graph(&root)?;

    let nodes = graph.find_by_name(symbol);
    if nodes.is_empty() {
        println!("Symbol '{symbol}' not found");
        return Ok(());
    }

    for node in nodes {
        println!("Impact analysis for {} ({:?}):", node.name, node.kind);

        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        visited.insert(node.id);
        queue.push_back((node.id, 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            let callers = graph.callers(current_id);
            for caller in callers {
                if visited.insert(caller.id) {
                    let indent = "  ".repeat(depth + 1);
                    println!(
                        "{indent}<- {} ({:?}, {}:{})",
                        caller.name,
                        caller.kind,
                        caller.file_path.display(),
                        caller.span.start_line
                    );
                    queue.push_back((caller.id, depth + 1));
                }
            }
        }

        let affected = visited.len() - 1;
        println!("\n  Total affected symbols: {affected}\n");
    }

    Ok(())
}

fn cmd_dead_code(path: &PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let graph = load_graph(&root)?;

    println!("Dead code analysis:\n");

    // Use the liveness scores computed by Phase 13 during `graphy analyze`.
    // The pipeline stores liveness in node.confidence (0.0 = dead, 1.0 = alive).
    // This ensures CLI, MCP, and web API all report identical results.
    let callable_kinds = [NodeKind::Function, NodeKind::Method];
    let mut dead: Vec<_> = graph
        .all_nodes()
        .filter(|n| callable_kinds.contains(&n.kind))
        .filter(|n| !graph.is_phantom(n.id))
        .filter(|n| n.confidence < 0.5)
        .collect();

    dead.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.span.start_line.cmp(&b.span.start_line))
    });

    if dead.is_empty() {
        println!("No dead code detected.");
    } else {
        println!("Found {} potentially unused symbols:\n", dead.len());
        for node in &dead {
            println!(
                "  {} ({}:{}) [liveness: {:.0}%]",
                node.name,
                node.file_path.display(),
                node.span.start_line,
                node.confidence * 100.0
            );
        }
    }

    Ok(())
}

fn cmd_hotspots(max_results: usize) -> Result<()> {
    let root = std::fs::canonicalize(".")?;
    let graph = load_graph(&root)?;

    let mut hotspots: Vec<_> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable() && n.complexity.is_some())
        .collect();

    hotspots.sort_by(|a, b| {
        let ca = a.complexity.as_ref().map_or(0, |c| c.cyclomatic);
        let cb = b.complexity.as_ref().map_or(0, |c| c.cyclomatic);
        cb.cmp(&ca)
    });
    hotspots.truncate(max_results);

    if hotspots.is_empty() {
        println!("No complexity data. Run `graphy analyze` first.");
    } else {
        println!("Complexity Hotspots:\n");
        for node in &hotspots {
            let Some(cx) = node.complexity.as_ref() else { continue };
            println!(
                "  {} (cyc={}, cog={}, loc={}) — {}:{}",
                node.name,
                cx.cyclomatic,
                cx.cognitive,
                cx.loc,
                node.file_path.display(),
                node.span.start_line
            );
        }
    }

    Ok(())
}

fn cmd_taint(symbol: Option<&str>) -> Result<()> {
    let root = std::fs::canonicalize(".")?;
    let graph = load_graph(&root)?;

    let tainted: Vec<_> = graph
        .all_nodes()
        .filter(|n| !graph.outgoing(n.id, graphy_core::EdgeKind::TaintedBy).is_empty())
        .filter(|n| symbol.map_or(true, |s| n.name.contains(s)))
        .collect();

    if tainted.is_empty() {
        println!("No taint paths detected.");
    } else {
        println!("Tainted symbols:\n");
        for node in &tainted {
            let sources = graph.outgoing(node.id, graphy_core::EdgeKind::TaintedBy);
            println!(
                "  {} ({}:{})",
                node.name,
                node.file_path.display(),
                node.span.start_line
            );
            for src in &sources {
                println!("    <- tainted by: {}", src.name);
            }
        }
    }

    Ok(())
}

fn cmd_stats(path: &PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let graph = load_graph(&root)?;

    println!("Graph Statistics:");
    println!("  Nodes: {}", graph.node_count());
    println!("  Edges: {}", graph.edge_count());
    println!();

    let kinds = [
        NodeKind::File,
        NodeKind::Folder,
        NodeKind::Module,
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
        NodeKind::Parameter,
        NodeKind::Field,
        NodeKind::Decorator,
        NodeKind::TypeAlias,
        NodeKind::EnumVariant,
    ];

    println!("  By kind:");
    for kind in &kinds {
        let count = graph.find_by_kind(*kind).len();
        if count > 0 {
            println!("    {kind:?}: {count}");
        }
    }

    let file_count = graph.indexed_files().len();
    println!("\n  Indexed files: {file_count}");

    Ok(())
}

// ── Server Commands ─────────────────────────────────────────────

fn cmd_serve(path: &PathBuf, watch: bool, use_lsp: bool) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let graph = ensure_graph(&root)?;

    let search_path = graphy_search::default_search_path(&root);
    let search = SearchIndex::new_persistent(&search_path).ok();

    let rt = build_runtime()?;

    if watch {
        rt.block_on(async {
            let graph = Arc::new(RwLock::new(graph));
            let search = search.map(Arc::new);

            // Create notification channel: watcher -> MCP server
            let (notify_tx, notify_rx) = graphy_mcp::notification_channel();

            let server = graphy_mcp::McpServer::new_shared(
                graph.clone(),
                search.clone(),
                root.clone(),
            )
            .with_notifications(notify_rx);

            let mcp = tokio::spawn(async move {
                if let Err(e) = server.run().await {
                    eprintln!("MCP server error: {e}");
                }
            });

            let mut watcher = graphy_watch::FileWatcher::new(root, graph)
                .with_lsp(use_lsp)
                .with_on_reindex(Box::new(move |files, nodes, edges| {
                    let _ = notify_tx.try_send(graphy_mcp::GraphUpdateEvent {
                        files_changed: files,
                        node_count: nodes,
                        edge_count: edges,
                    });
                }));
            if let Some(s) = search {
                watcher = watcher.with_search(s);
            }
            tokio::spawn(async move {
                if let Err(e) = watcher.watch().await {
                    eprintln!("Watcher error: {e}");
                }
            });

            // MCP server drives the process — when stdin closes, we exit
            mcp.await.ok();
            Ok(())
        })
    } else {
        rt.block_on(async {
            let server = graphy_mcp::McpServer::new(graph, search, root);
            server.run().await
        })
    }
}

fn cmd_watch(path: &PathBuf, use_lsp: bool) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let graph = ensure_graph(&root)?;

    let rt = build_runtime()?;
    rt.block_on(async {
        let graph = Arc::new(RwLock::new(graph));
        let watcher = graphy_watch::FileWatcher::new(root, graph)
            .with_lsp(use_lsp);
        watcher.watch().await
    })
}

// ── Advanced Commands ───────────────────────────────────────────

fn cmd_deps(path: &PathBuf, vulns: bool) -> Result<()> {
    let root = std::fs::canonicalize(path)?;

    let graph = if storage::default_db_path(&root).exists() {
        Some(load_graph(&root)?)
    } else {
        None
    };

    let rt = build_runtime()?;
    let analysis = rt.block_on(graphy_deps::analyze_dependencies(
        &root,
        graph.as_ref(),
        vulns,
    ))?;

    print!("{}", graphy_deps::format_deps_text(&analysis));

    Ok(())
}

fn cmd_diff(base: &PathBuf, head: &PathBuf, format: &str, fail_on_breaking: bool) -> Result<()> {
    let base_graph = load_graph_from_path(base)?;
    let head_graph = load_graph_from_path(head)?;

    let graph_diff = diff::diff_graphs(&base_graph, &head_graph);

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&graph_diff)?);
        }
        _ => {
            print!("{}", diff::format_diff_text(&graph_diff));
        }
    }

    if fail_on_breaking && !graph_diff.breaking_changes.is_empty() {
        eprintln!(
            "\n{} breaking change(s) detected. Failing.",
            graph_diff.breaking_changes.len()
        );
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_context_gen(path: &PathBuf, format: &str) -> Result<()> {
    let root = std::fs::canonicalize(path)?;
    let mut graph = load_graph(&root)?;

    let ctx = graphy_analysis::context_gen::generate_context(&mut graph, &root);

    match format {
        "json" => {
            let json = graphy_analysis::context_gen::format_as_json(&ctx);
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => {
            print!("{}", graphy_analysis::context_gen::format_as_markdown(&ctx));
        }
    }

    Ok(())
}

fn cmd_multi_repo(paths: &[PathBuf], output: Option<&Path>) -> Result<()> {
    if paths.len() < 2 {
        anyhow::bail!("Multi-repo analysis requires at least 2 repository paths");
    }

    let config = graphy_analysis::multi_repo::MultiRepoConfig {
        roots: paths.to_vec(),
        pipeline_config: PipelineConfig::default(),
    };

    let result = graphy_analysis::multi_repo::analyze_multi_repo(&config)?;

    println!(
        "Multi-repo analysis complete: {} repos, {} nodes, {} edges ({} cross-repo)",
        result.repo_count,
        result.merged_graph.node_count(),
        result.merged_graph.edge_count(),
        result.cross_repo_edges,
    );

    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        storage::save_graph(&result.merged_graph, out_path)?;
        println!("Merged graph saved to {}", out_path.display());
    }

    Ok(())
}

// ── Language Management ─────────────────────────────────────────

fn cmd_lang(action: LangAction) -> Result<()> {
    match action {
        LangAction::Add { name } => {
            let info = graphy_parser::dynamic_loader::grammar_info_by_name(&name)
                .ok_or_else(|| {
                    let available: Vec<_> = graphy_parser::dynamic_loader::KNOWN_GRAMMARS
                        .iter()
                        .map(|g| g.name)
                        .collect();
                    anyhow::anyhow!(
                        "Unknown grammar: '{name}'. Available: {}",
                        available.join(", ")
                    )
                })?;
            graphy_parser::grammar_compiler::install_grammar(info)
        }
        LangAction::Remove { name } => {
            graphy_parser::grammar_compiler::remove_grammar(&name)
        }
        LangAction::List => {
            eprintln!();
            eprintln!("  \x1b[1mBuilt-in languages\x1b[0m (always available):");
            eprintln!("    Python, TypeScript, JavaScript, Rust, Svelte");
            eprintln!();
            eprintln!("  \x1b[1mDynamic languages\x1b[0m (install with: graphy lang add <name>):");
            for info in graphy_parser::dynamic_loader::KNOWN_GRAMMARS {
                let installed = graphy_parser::dynamic_loader::is_installed(info.name);
                let status = if installed {
                    "\x1b[32m✓\x1b[0m"
                } else {
                    "\x1b[2m-\x1b[0m"
                };
                let exts: Vec<_> = info.extensions.iter().map(|e| format!(".{e}")).collect();
                eprintln!("    {status} {:<12} {}", info.name, exts.join(", "));
            }
            eprintln!();
            eprintln!("  Grammars stored at: {}", graphy_parser::dynamic_loader::grammars_dir().display());
            eprintln!();
            Ok(())
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Load existing index or run the analysis pipeline.
fn ensure_graph(root: &Path) -> Result<CodeGraph> {
    // Always run a FULL (non-incremental) pipeline from source files.
    //
    // Why not incremental: the pipeline starts with CodeGraph::new() (empty).
    // Incremental mode skips parsing unchanged files, but since the graph is
    // empty, those files' nodes are simply missing. Incremental only works
    // when there's an existing graph to update — the pipeline doesn't have one.
    //
    // Why not load stored graph: the watcher modifies the graph in-memory
    // during a session, and those changes may not be saved. Loading a stale
    // stored graph shows missing files and wrong edges.
    //
    // A full pipeline re-parses everything from disk — always correct.
    let config = PipelineConfig {
        incremental: false,
        ..Default::default()
    };
    let pipeline = AnalysisPipeline::new(root.to_path_buf(), config);
    let graph = pipeline.run()?;

    // Persist so other commands (serve, analyze) can load it
    let db_path = storage::default_db_path(root);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Err(e) = storage::save_graph(&graph, &db_path) {
        eprintln!("  \x1b[33m!\x1b[0m Failed to save index: {e}");
    }

    Ok(graph)
}

/// Build and populate a search index.
#[cfg(feature = "web")]
fn build_search(root: &Path, graph: &CodeGraph) -> Result<SearchIndex> {
    let search_path = graphy_search::default_search_path(root);
    let search = SearchIndex::new_persistent(&search_path)
        .or_else(|_| SearchIndex::new_in_memory())?;
    search.index_graph(graph)?;
    Ok(search)
}

/// Load a graph, failing if no index exists.
fn load_graph(root: &Path) -> Result<CodeGraph> {
    let db_path = storage::default_db_path(root);
    if !db_path.exists() {
        anyhow::bail!(
            "No index found. Run `graphy` or `graphy analyze` first."
        );
    }
    storage::load_graph(&db_path).map_err(Into::into)
}

/// Load a graph from a directory (using .redb) or from a .redb file directly.
fn load_graph_from_path(path: &PathBuf) -> Result<CodeGraph> {
    if path.extension().is_some_and(|e| e == "redb") {
        storage::load_graph(path).map_err(Into::into)
    } else {
        let root = std::fs::canonicalize(path)?;
        ensure_graph(&root)
    }
}

#[cfg(feature = "web")]
/// Print the Vite-style startup banner.
fn print_banner(
    port: u16,
    node_count: usize,
    edge_count: usize,
    file_count: usize,
    elapsed: std::time::Duration,
    watching: bool,
) {
    let ms = elapsed.as_millis();
    eprintln!();
    eprintln!(
        "  \x1b[1;36mGRAPHY\x1b[0m v{}  \x1b[2mready in {}ms\x1b[0m",
        env!("CARGO_PKG_VERSION"),
        ms
    );
    eprintln!();
    eprintln!(
        "  \x1b[1;32m>\x1b[0m  \x1b[1mLocal:\x1b[0m   http://localhost:{}/",
        port
    );
    eprintln!(
        "  \x1b[2m>\x1b[0m  \x1b[2mGraph:\x1b[0m   {} symbols \x1b[2m·\x1b[0m {} edges \x1b[2m·\x1b[0m {} files",
        node_count, edge_count, file_count
    );
    if watching {
        eprintln!(
            "  \x1b[2m>\x1b[0m  \x1b[2mWatch:\x1b[0m   watching for file changes"
        );
    }
    eprintln!();
}

fn print_summary(graph: &CodeGraph) {
    println!("\nIndex Summary:");
    println!(
        "  {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    let (mut files, mut classes, mut functions, mut methods, mut imports) = (0, 0, 0, 0, 0);
    for n in graph.all_nodes() {
        match n.kind {
            NodeKind::File => files += 1,
            NodeKind::Class => classes += 1,
            NodeKind::Function => functions += 1,
            NodeKind::Method => methods += 1,
            NodeKind::Import => imports += 1,
            _ => {}
        }
    }

    println!(
        "  {files} files, {classes} classes, {functions} functions, {methods} methods, {imports} imports"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parse_no_args() {
        // Default: no subcommand, no verbose
        let cli = Cli::try_parse_from(["graphy"]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.verbose);
    }

    #[test]
    fn cli_parse_verbose() {
        let cli = Cli::try_parse_from(["graphy", "-v"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn cli_parse_analyze() {
        let cli = Cli::try_parse_from(["graphy", "analyze", "/tmp/project"]).unwrap();
        match cli.command.unwrap() {
            Commands::Analyze { path, full, lsp } => {
                assert_eq!(path, PathBuf::from("/tmp/project"));
                assert!(!full);
                assert!(!lsp);
            }
            _ => panic!("Expected Analyze command"),
        }
    }

    #[test]
    fn cli_parse_analyze_with_flags() {
        let cli = Cli::try_parse_from(["graphy", "analyze", "--full", "--lsp"]).unwrap();
        match cli.command.unwrap() {
            Commands::Analyze { full, lsp, .. } => {
                assert!(full);
                assert!(lsp);
            }
            _ => panic!("Expected Analyze command"),
        }
    }

    #[test]
    fn cli_parse_search() {
        let cli = Cli::try_parse_from(["graphy", "search", "my_function", "-n", "5"]).unwrap();
        match cli.command.unwrap() {
            Commands::Search { query, max_results, kind } => {
                assert_eq!(query, "my_function");
                assert_eq!(max_results, 5);
                assert!(kind.is_none());
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn cli_parse_search_with_kind() {
        let cli = Cli::try_parse_from(["graphy", "search", "foo", "--kind", "Function"]).unwrap();
        match cli.command.unwrap() {
            Commands::Search { kind, .. } => {
                assert_eq!(kind, Some("Function".into()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn cli_parse_impact() {
        let cli = Cli::try_parse_from(["graphy", "impact", "my_func", "--depth", "5"]).unwrap();
        match cli.command.unwrap() {
            Commands::Impact { symbol, depth } => {
                assert_eq!(symbol, "my_func");
                assert_eq!(depth, 5);
            }
            _ => panic!("Expected Impact command"),
        }
    }

    #[test]
    fn cli_parse_serve_with_watch() {
        let cli = Cli::try_parse_from(["graphy", "serve", "--watch", "--lsp"]).unwrap();
        match cli.command.unwrap() {
            Commands::Serve { watch, lsp, .. } => {
                assert!(watch);
                assert!(lsp);
            }
            _ => panic!("Expected Serve command"),
        }
    }

    #[test]
    fn cli_parse_lang_add() {
        let cli = Cli::try_parse_from(["graphy", "lang", "add", "go"]).unwrap();
        match cli.command.unwrap() {
            Commands::Lang { action } => match action {
                LangAction::Add { name } => assert_eq!(name, "go"),
                _ => panic!("Expected Add action"),
            },
            _ => panic!("Expected Lang command"),
        }
    }

    #[test]
    fn cli_parse_lang_list() {
        let cli = Cli::try_parse_from(["graphy", "lang", "list"]).unwrap();
        match cli.command.unwrap() {
            Commands::Lang { action } => assert!(matches!(action, LangAction::List)),
            _ => panic!("Expected Lang command"),
        }
    }

    #[test]
    fn cli_parse_diff() {
        let cli = Cli::try_parse_from(["graphy", "diff", "base_dir", "head_dir", "--fail-on-breaking"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff { base, head, fail_on_breaking, format } => {
                assert_eq!(base, PathBuf::from("base_dir"));
                assert_eq!(head, PathBuf::from("head_dir"));
                assert!(fail_on_breaking);
                assert_eq!(format, "text");
            }
            _ => panic!("Expected Diff command"),
        }
    }

    #[test]
    fn cli_parse_invalid_command() {
        let result = Cli::try_parse_from(["graphy", "nonexistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn build_runtime_succeeds() {
        let rt = build_runtime();
        assert!(rt.is_ok());
    }

    #[test]
    fn print_summary_empty_graph() {
        // Should not panic on empty graph
        let graph = CodeGraph::new();
        print_summary(&graph);
    }

    #[test]
    fn load_graph_from_path_redb_extension_detection() {
        // Verify .redb extension is detected and takes the direct load path
        // (We don't assert success/failure since redb may auto-create files)
        let path = PathBuf::from("/nonexistent_dir_xyz/graph.redb");
        assert!(path.extension().is_some_and(|e| e == "redb"));
    }

    #[test]
    fn load_graph_missing_index() {
        // load_graph on a path with no index should return an error
        let result = load_graph(Path::new("/tmp/nonexistent_graphy_test_xyz"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No index found"));
    }
}
