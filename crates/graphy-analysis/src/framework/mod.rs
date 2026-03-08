//! Framework plugin system for language-agnostic framework detection.
//!
//! Frameworks are defined as TOML config files in `config/frameworks/`.
//! Each file describes detection rules, string dispatch patterns,
//! convention entries, and entry decorators.
//!
//! Adding a new framework:
//! 1. Create `config/frameworks/my_framework.toml`
//! 2. Define name, languages, detect rules, and patterns
//! 3. No recompilation needed

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphy_core::{
    CodeGraph, EdgeKind, GirEdge, GirNode, Language, NodeKind, Span, SymbolId, Visibility,
};
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, info, warn};

pub(crate) mod detect;

// ── TOML schema ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FrameworkConfig {
    name: String,
    languages: Vec<String>,
    detect: DetectConfig,
    #[serde(default)]
    entry_decorators: Vec<String>,
    #[serde(default)]
    string_dispatches: Vec<StringDispatchConfig>,
    #[serde(default)]
    convention_entries: Vec<ConventionEntryConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct DetectConfig {
    #[serde(default)]
    npm_dep: Option<String>,
    #[serde(default)]
    pip_dep: Option<String>,
    #[serde(default)]
    cargo_dep: Option<String>,
    #[serde(default)]
    composer_dep: Option<String>,
    #[serde(default)]
    gem_dep: Option<String>,
    #[serde(default)]
    gradle_or_maven_dep: Option<String>,
    #[serde(rename = "import", default)]
    import_name: Option<String>,
    #[serde(default)]
    config_files: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    dirs: Vec<String>,
    #[serde(default)]
    file_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StringDispatchConfig {
    pattern: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ConventionEntryConfig {
    file_pattern: String,
    #[serde(default)]
    exported_only: bool,
    reason: String,
}

// ── Public data structures ─────────────────────────────────

/// A string-dispatch pattern: a function call that routes to another function
/// via a string argument (e.g., WordPress `add_action('hook', 'func')`).
#[derive(Debug, Clone)]
pub struct StringDispatch {
    /// Regex with capture group 1 = target function name.
    pub pattern: String,
    /// Human-readable description.
    pub description: String,
}

/// A file naming convention that marks contained functions as framework entry points.
#[derive(Debug, Clone)]
pub struct ConventionEntry {
    /// File pattern. `*suffix` = suffix match, `path/` = contains, else exact name.
    pub file_pattern: String,
    /// If true, only exported/public functions get the boost.
    pub exported_only: bool,
    /// Reason string for dead code detection.
    pub reason: String,
}

/// Result of framework analysis.
#[derive(Debug, Default)]
pub struct FrameworkResult {
    pub frameworks_detected: Vec<String>,
    pub annotations_added: usize,
}

// ── Loaded plugin (from TOML) ──────────────────────────────

struct LoadedPlugin {
    name: String,
    languages: Vec<Language>,
    detect: DetectConfig,
    /// Informational: decorator names that mark entry points (used by dead code heuristics).
    #[allow(dead_code)]
    entry_decorators: Vec<String>,
    string_dispatches: Vec<StringDispatch>,
    convention_entries: Vec<ConventionEntry>,
}

impl LoadedPlugin {
    fn from_config(config: FrameworkConfig) -> Self {
        let languages: Vec<Language> = config
            .languages
            .iter()
            .filter_map(|s| parse_language(s))
            .collect();

        let string_dispatches = config
            .string_dispatches
            .into_iter()
            .map(|sd| StringDispatch {
                pattern: sd.pattern,
                description: sd.description,
            })
            .collect();

        let convention_entries = config
            .convention_entries
            .into_iter()
            .map(|ce| ConventionEntry {
                file_pattern: ce.file_pattern,
                exported_only: ce.exported_only,
                reason: ce.reason,
            })
            .collect();

        Self {
            name: config.name,
            languages,
            detect: config.detect,
            entry_decorators: config.entry_decorators,
            string_dispatches,
            convention_entries,
        }
    }

    fn is_detected(&self, graph: &CodeGraph, root: &Path) -> bool {
        let d = &self.detect;

        // Check package manager dependencies
        if let Some(dep) = &d.npm_dep {
            if detect::has_npm_dep(root, dep) {
                return true;
            }
        }
        if let Some(dep) = &d.pip_dep {
            if detect::has_pip_dep(root, dep) {
                return true;
            }
        }
        if let Some(dep) = &d.cargo_dep {
            if detect::has_cargo_dep(root, dep) {
                return true;
            }
        }
        if let Some(dep) = &d.composer_dep {
            if detect::has_composer_dep(root, dep) {
                return true;
            }
        }
        if let Some(dep) = &d.gem_dep {
            if detect::has_gem_dep(root, dep) {
                return true;
            }
        }
        if let Some(dep) = &d.gradle_or_maven_dep {
            if detect::has_gradle_or_maven_dep(root, dep) {
                return true;
            }
        }

        // Check import in graph
        if let Some(module) = &d.import_name {
            if detect::has_import_of(graph, module) {
                return true;
            }
        }

        // Check config files exist
        for cf in &d.config_files {
            if root.join(cf).exists() {
                return true;
            }
        }

        // Check specific files exist
        for f in &d.files {
            if root.join(f).exists() {
                return true;
            }
        }

        // Check directories exist
        for dir in &d.dirs {
            if root.join(dir).is_dir() {
                return true;
            }
        }

        // Check file names exist anywhere (walk is expensive, use simple glob)
        for name in &d.file_names {
            if root.join(name).exists() {
                return true;
            }
        }

        false
    }
}

fn parse_language(s: &str) -> Option<Language> {
    match s {
        "Python" => Some(Language::Python),
        "TypeScript" => Some(Language::TypeScript),
        "JavaScript" => Some(Language::JavaScript),
        "Rust" => Some(Language::Rust),
        "Go" => Some(Language::Go),
        "Java" => Some(Language::Java),
        "Kotlin" => Some(Language::Kotlin),
        "Php" | "PHP" => Some(Language::Php),
        "Ruby" => Some(Language::Ruby),
        "C" => Some(Language::C),
        "Cpp" | "C++" => Some(Language::Cpp),
        "CSharp" | "C#" => Some(Language::CSharp),
        "Svelte" => Some(Language::Svelte),
        _ => {
            warn!("Unknown language in framework config: {s}");
            None
        }
    }
}

// ── Built-in TOML configs (embedded at compile time) ───────

const BUILTIN_CONFIGS: &[(&str, &str)] = &[
    ("wordpress", include_str!("../../frameworks/wordpress.toml")),
    ("react", include_str!("../../frameworks/react.toml")),
    ("nextjs", include_str!("../../frameworks/nextjs.toml")),
    ("express", include_str!("../../frameworks/express.toml")),
    ("flask", include_str!("../../frameworks/flask.toml")),
    ("django", include_str!("../../frameworks/django.toml")),
    ("fastapi", include_str!("../../frameworks/fastapi.toml")),
    ("laravel", include_str!("../../frameworks/laravel.toml")),
    ("spring", include_str!("../../frameworks/spring.toml")),
    ("rails", include_str!("../../frameworks/rails.toml")),
    ("angular", include_str!("../../frameworks/angular.toml")),
    ("vue", include_str!("../../frameworks/vue.toml")),
    ("nestjs", include_str!("../../frameworks/nestjs.toml")),
    ("sveltekit", include_str!("../../frameworks/sveltekit.toml")),
    ("axum", include_str!("../../frameworks/axum.toml")),
];

// ── Registry ───────────────────────────────────────────────

/// Registry of all framework plugins. Detects and applies framework-specific
/// patterns during analysis.
pub struct FrameworkRegistry {
    plugins: Vec<LoadedPlugin>,
}

impl FrameworkRegistry {
    /// Create a new registry with all built-in framework plugins.
    pub fn new() -> Self {
        let mut plugins = Vec::new();

        // Load built-in configs (embedded at compile time)
        for (name, toml_str) in BUILTIN_CONFIGS {
            match toml::from_str::<FrameworkConfig>(toml_str) {
                Ok(config) => plugins.push(LoadedPlugin::from_config(config)),
                Err(e) => warn!("Failed to parse built-in framework config '{name}': {e}"),
            }
        }

        Self { plugins }
    }

    /// Create a registry that also loads custom framework configs from a directory.
    pub fn with_custom_dir(mut self, dir: &Path) -> Self {
        if !dir.is_dir() {
            return self;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "toml") {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => match toml::from_str::<FrameworkConfig>(&content) {
                            Ok(config) => {
                                debug!("Loaded custom framework: {}", config.name);
                                self.plugins.push(LoadedPlugin::from_config(config));
                            }
                            Err(e) => warn!("Failed to parse {}: {e}", path.display()),
                        },
                        Err(e) => warn!("Failed to read {}: {e}", path.display()),
                    }
                }
            }
        }
        self
    }

    /// Detect active frameworks and apply their patterns to the graph.
    /// Creates synthetic AnnotatedWith edges so dead code detection
    /// recognizes framework-managed functions as alive.
    pub fn analyze(&self, graph: &mut CodeGraph, root: &Path) -> FrameworkResult {
        let mut result = FrameworkResult::default();

        // Detect active frameworks
        let active_indices: Vec<usize> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_detected(graph, root))
            .map(|(i, _)| i)
            .collect();

        for &i in &active_indices {
            result
                .frameworks_detected
                .push(self.plugins[i].name.clone());
        }

        if active_indices.is_empty() {
            return result;
        }

        let mut final_annotations: Vec<(SymbolId, String, PathBuf, Language)> = Vec::new();

        // 1. String dispatch resolution
        let file_contents = load_source_files(graph);

        for &i in &active_indices {
            let plugin = &self.plugins[i];
            for dispatch in &plugin.string_dispatches {
                let Ok(re) = Regex::new(&dispatch.pattern) else {
                    debug!("Invalid regex in {}: {}", dispatch.description, dispatch.pattern);
                    continue;
                };
                for (_path, content) in &file_contents {
                    for caps in re.captures_iter(content) {
                        if let Some(m) = caps.get(1) {
                            if let Some(target) = graph
                                .find_by_name(m.as_str())
                                .into_iter()
                                .find(|n| n.kind.is_callable() && plugin.languages.contains(&n.language))
                            {
                                final_annotations.push((
                                    target.id,
                                    format!("{}_hook", plugin.name.to_lowercase()),
                                    target.file_path.clone(),
                                    target.language,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // 2. Convention entries
        let all_callables: Vec<_> = graph
            .all_nodes()
            .filter(|n| n.kind.is_callable())
            .map(|n| (n.id, n.file_path.clone(), n.language, n.visibility))
            .collect();

        for &i in &active_indices {
            let plugin = &self.plugins[i];
            let plugin_name = plugin.name.to_lowercase();
            for conv in &plugin.convention_entries {
                for (id, file_path, lang, vis) in &all_callables {
                    if !matches_file_pattern(file_path, &conv.file_pattern) {
                        continue;
                    }
                    if conv.exported_only
                        && *vis != Visibility::Exported
                        && *vis != Visibility::Public
                    {
                        continue;
                    }
                    final_annotations.push((
                        *id,
                        format!("{}_{}", plugin_name, conv.reason),
                        file_path.clone(),
                        *lang,
                    ));
                }
            }
        }

        // 3. Apply all annotations as synthetic AnnotatedWith edges
        for (target_id, decorator_name, file_path, language) in final_annotations.iter() {
            let decorator = GirNode::new(
                decorator_name.to_string(),
                NodeKind::Decorator,
                file_path.to_path_buf(),
                Span::new(0, 0, 0, 0),
                *language,
            );
            let dec_id = decorator.id;
            graph.add_node(decorator);
            graph.add_edge(
                *target_id,
                dec_id,
                GirEdge::new(EdgeKind::AnnotatedWith),
            );
        }

        result.annotations_added = final_annotations.len();

        info!(
            "Phase 5.8 (Frameworks): detected [{}], {} annotations",
            result.frameworks_detected.join(", "),
            result.annotations_added,
        );

        result
    }
}

/// Match a file path against a convention entry pattern.
fn matches_file_pattern(file_path: &Path, pattern: &str) -> bool {
    let file_name = file_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_str = file_path.to_string_lossy();

    if pattern.starts_with('*') {
        // Suffix match: *.tsx, *Controller.php
        file_name.ends_with(&pattern[1..])
    } else if pattern.ends_with('/') {
        // Path contains: app/controllers/
        file_str.contains(pattern)
    } else {
        // Exact file name: page.tsx, functions.php
        file_name == pattern
    }
}

fn load_source_files(graph: &CodeGraph) -> HashMap<PathBuf, String> {
    let mut files = HashMap::new();
    let paths: Vec<PathBuf> = graph
        .all_nodes()
        .filter(|n| n.kind == NodeKind::File)
        .map(|n| n.file_path.clone())
        .collect();
    for path in paths {
        if let Ok(content) = std::fs::read_to_string(&path) {
            files.insert(path, content);
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_creates_all_plugins() {
        let registry = FrameworkRegistry::new();
        assert!(registry.plugins.len() >= 15);
    }

    #[test]
    fn empty_graph_detects_no_frameworks() {
        let registry = FrameworkRegistry::new();
        let mut graph = CodeGraph::new();
        let result = registry.analyze(&mut graph, Path::new("/nonexistent"));
        assert!(result.frameworks_detected.is_empty());
        assert_eq!(result.annotations_added, 0);
    }

    #[test]
    fn file_pattern_matching() {
        // Suffix match
        assert!(matches_file_pattern(Path::new("src/UserController.php"), "*Controller.php"));
        assert!(!matches_file_pattern(Path::new("src/utils.php"), "*Controller.php"));

        // Exact match
        assert!(matches_file_pattern(Path::new("theme/functions.php"), "functions.php"));
        assert!(!matches_file_pattern(Path::new("theme/utils.php"), "functions.php"));

        // Extension match
        assert!(matches_file_pattern(Path::new("src/App.tsx"), "*.tsx"));
        assert!(!matches_file_pattern(Path::new("src/App.ts"), "*.tsx"));

        // Path contains
        assert!(matches_file_pattern(Path::new("/proj/app/controllers/user.rb"), "app/controllers/"));
        assert!(!matches_file_pattern(Path::new("/proj/lib/user.rb"), "app/controllers/"));
    }

    #[test]
    fn toml_config_parses_correctly() {
        let toml_str = include_str!("../../frameworks/flask.toml");
        let config: FrameworkConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "Flask");
        assert_eq!(config.languages, vec!["Python"]);
        assert_eq!(config.entry_decorators.len(), 3);
    }

    #[test]
    fn all_builtin_configs_parse() {
        for (name, toml_str) in BUILTIN_CONFIGS {
            let config: FrameworkConfig = toml::from_str(toml_str)
                .unwrap_or_else(|e| panic!("Failed to parse {name}.toml: {e}"));
            assert!(!config.name.is_empty(), "{name} has empty name");
            assert!(!config.languages.is_empty(), "{name} has no languages");
        }
    }

    #[test]
    fn plugin_with_unknown_languages_filters_them_out() {
        // A framework config listing unrecognized language strings should not
        // panic — parse_language returns None and they are silently dropped.
        let toml_str = r#"
name = "FakeFramework"
languages = ["BrainFuck", "Haskell", "Python"]

[detect]
files = ["fake.lock"]
"#;
        let config: FrameworkConfig = toml::from_str(toml_str).unwrap();
        let plugin = LoadedPlugin::from_config(config);

        // Only "Python" should survive; the two unknown languages are filtered out
        assert_eq!(plugin.languages.len(), 1);
        assert_eq!(plugin.languages[0], Language::Python);

        // The plugin should still be functional — is_detected returns false
        // on a nonexistent path without panicking
        let graph = CodeGraph::new();
        assert!(!plugin.is_detected(&graph, Path::new("/nonexistent")));
    }
}
