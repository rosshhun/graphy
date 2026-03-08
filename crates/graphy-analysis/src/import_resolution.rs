//! Phase 4: Resolve imports to actual files and symbols.
//!
//! Language-aware: applies Python dotted-module resolution for .py files,
//! relative-path resolution for JS/TS, and Rust `use` resolution for .rs files.
//! Builds a module map then resolves Import nodes to their target File/symbol nodes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphy_core::{
    CodeGraph, EdgeKind, EdgeMetadata, GirEdge, Language, NodeKind, SymbolId,
};
use petgraph::Direction;
use tracing::{debug, trace};

/// Build a mapping from module path to the File node's SymbolId.
/// Supports Python dotted paths and JS/TS relative paths.
fn build_module_map(graph: &CodeGraph, root: &Path) -> HashMap<String, SymbolId> {
    let mut map = HashMap::new();

    for node in graph.find_by_kind(NodeKind::File) {
        if let Ok(rel) = node.file_path.strip_prefix(root) {
            // Python: dotted module path
            let py_module = path_to_python_module(rel);
            if !py_module.is_empty() {
                map.insert(py_module, node.id);
            }

            // JS/TS: relative path without extension (e.g., "src/utils/math")
            let rel_str = rel.to_string_lossy();
            let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts") {
                // Store with and without extension for flexible matching
                let without_ext = rel_str.trim_end_matches(&format!(".{}", ext));
                map.insert(without_ext.to_string(), node.id);
                // Also store the /index variant for directory imports
                if without_ext.ends_with("/index") {
                    let dir = without_ext.trim_end_matches("/index");
                    map.insert(dir.to_string(), node.id);
                }
            }
        }
    }

    map
}

/// Convert a relative file path to a Python dotted module path.
///   e.g. utils/math.py  -> "utils.math"
///        utils/__init__.py -> "utils"
fn path_to_python_module(rel: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();

    for component in rel.components() {
        if let std::path::Component::Normal(s) = component {
            parts.push(s.to_str().unwrap_or(""));
        }
    }

    if parts.is_empty() {
        return String::new();
    }

    // Remove .py extension from last part
    let last = parts.last().copied().unwrap_or("");
    if last == "__init__.py" {
        parts.pop();
    } else if let Some(stem) = last.strip_suffix(".py") {
        let len = parts.len();
        parts[len - 1] = stem;
    } else {
        // Not a Python file
        return String::new();
    }

    parts.join(".")
}

/// Resolve the module path for a relative import.
///   `from . import foo`  => sibling module "foo"
///   `from ..bar import baz` => parent.bar
fn resolve_relative_import(
    importing_file: &Path,
    root: &Path,
    module_name: &str,
) -> Option<String> {
    let rel = importing_file.strip_prefix(root).ok()?;
    let current_module = path_to_python_module(rel);
    let mut parts: Vec<&str> = current_module.split('.').collect();

    // Remove the file's own module name — relative imports are relative to the
    // PACKAGE (directory), not the module file itself.
    if !parts.is_empty() {
        parts.pop();
    }

    // Count leading dots
    let dots = module_name.chars().take_while(|c| *c == '.').count();
    let remainder = &module_name[dots..];

    if dots == 0 {
        return None; // Not a relative import
    }

    // Go up `dots - 1` levels from the current package
    // (one dot = current package, two dots = parent package, etc.)
    let levels_up = dots.saturating_sub(1);
    let depth = parts.len().saturating_sub(levels_up);
    let base: Vec<&str> = parts[..depth].to_vec();

    let mut result = base.join(".");
    if !remainder.is_empty() {
        if !result.is_empty() {
            result.push('.');
        }
        result.push_str(remainder);
    }

    Some(result)
}

/// Build a mapping from Rust crate names to their `src/` directory paths.
/// Reads workspace Cargo.toml and member Cargo.toml files.
/// Normalizes crate names: `graphy-core` -> `graphy_core`.
fn build_rust_crate_map(root: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();

    let workspace_toml_path = root.join("Cargo.toml");
    let workspace_content = match std::fs::read_to_string(&workspace_toml_path) {
        Ok(c) => c,
        Err(_) => return map,
    };

    let workspace_toml: toml::Value = match workspace_content.parse() {
        Ok(v) => v,
        Err(_) => return map,
    };

    // Collect workspace member glob patterns
    let members = workspace_toml
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    // Also handle non-workspace single crate
    if members.is_empty() {
        if let Some(name) = workspace_toml
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            let normalized = name.replace('-', "_");
            let src_dir = root.join("src");
            if src_dir.exists() {
                map.insert(normalized, src_dir);
            }
        }
        return map;
    }

    for member_pattern in &members {
        let pattern_str = member_pattern.as_str().unwrap_or("");
        if pattern_str.is_empty() {
            continue;
        }

        // Resolve glob patterns (e.g., "crates/*")
        let full_pattern = root.join(pattern_str);
        let paths = glob_member_dirs(&full_pattern);

        for member_dir in paths {
            let member_toml_path = member_dir.join("Cargo.toml");
            let member_content = match std::fs::read_to_string(&member_toml_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let member_toml: toml::Value = match member_content.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(name) = member_toml
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                let normalized = name.replace('-', "_");
                let src_dir = member_dir.join("src");
                if src_dir.exists() {
                    map.insert(normalized, src_dir);
                }
            }
        }
    }

    map
}

/// Expand a workspace member pattern (handles `*` glob) into actual directories.
fn glob_member_dirs(pattern: &Path) -> Vec<PathBuf> {
    let pattern_str = pattern.to_string_lossy();
    if pattern_str.contains('*') {
        // Simple glob: replace `*` with directory listing
        let parent = pattern.parent().unwrap_or(pattern);
        if let Ok(entries) = std::fs::read_dir(parent) {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join("Cargo.toml").exists())
                .collect()
        } else {
            vec![]
        }
    } else if pattern.is_dir() {
        vec![pattern.to_path_buf()]
    } else {
        vec![]
    }
}

/// Phase 4: Resolve imports in the graph.
pub fn resolve_imports(graph: &mut CodeGraph, root: &Path) {
    let module_map = build_module_map(graph, root);
    let rust_crate_map = build_rust_crate_map(root);

    // Also build a name -> SymbolId map for symbol-level resolution
    let mut symbol_map: HashMap<(String, String), SymbolId> = HashMap::new(); // (module, name) -> id
    for node in graph.all_nodes() {
        if matches!(
            node.kind,
            NodeKind::Function
                | NodeKind::Method
                | NodeKind::Class
                | NodeKind::Constant
                | NodeKind::Variable
                | NodeKind::Constructor
        ) {
            if let Ok(rel) = node.file_path.strip_prefix(root) {
                // Python modules
                let py_module = path_to_python_module(rel);
                if !py_module.is_empty() {
                    symbol_map.insert((py_module, node.name.clone()), node.id);
                }
                // JS/TS modules: use relative path without extension
                let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") {
                    let rel_str = rel.to_string_lossy();
                    let without_ext = rel_str.trim_end_matches(&format!(".{}", ext));
                    symbol_map.insert((without_ext.to_string(), node.name.clone()), node.id);
                }
            }
        }
    }

    // Collect all Import nodes with their edge info
    let import_nodes: Vec<(SymbolId, PathBuf, String, Option<String>, Language)> = graph
        .find_by_kind(NodeKind::Import)
        .iter()
        .map(|n| {
            let sig = n.signature.clone().unwrap_or_default();
            (n.id, n.file_path.clone(), n.name.clone(), Some(sig), n.language)
        })
        .collect();

    // Collect edges to add (we can't mutate graph while iterating)
    let mut edges_to_add: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();

    for (import_id, file_path, module_name, sig_opt, lang) in &import_nodes {
        let sig = sig_opt.as_deref().unwrap_or("");

        // Only apply Python-style dotted module resolution to Python imports
        if *lang != Language::Python {
            // For JS/TS: try to resolve relative path imports
            if matches!(lang, Language::JavaScript | Language::TypeScript | Language::Svelte) {
                if let Some(resolved_path) = resolve_js_import(file_path, root, module_name) {
                    if let Some(&target_id) = module_map.get(&resolved_path) {
                        let items = get_import_items(graph, *import_id);
                        if items.is_empty() {
                            let edge = GirEdge::new(EdgeKind::Imports).with_confidence(0.8);
                            edges_to_add.push((*import_id, target_id, edge));
                        } else {
                            for item in &items {
                                if let Some(&sym_id) = symbol_map.get(&(resolved_path.clone(), item.clone())) {
                                    let edge = GirEdge::new(EdgeKind::ImportsFrom)
                                        .with_confidence(1.0)
                                        .with_metadata(EdgeMetadata::Import {
                                            alias: None,
                                            items: vec![item.clone()],
                                        });
                                    edges_to_add.push((*import_id, sym_id, edge));
                                } else {
                                    let edge = GirEdge::new(EdgeKind::ImportsFrom)
                                        .with_confidence(0.7)
                                        .with_metadata(EdgeMetadata::Import {
                                            alias: None,
                                            items: vec![item.clone()],
                                        });
                                    edges_to_add.push((*import_id, target_id, edge));
                                }
                            }
                        }
                    }
                }
            }

            // For Rust: resolve `use crate_name::symbol` cross-crate imports
            if *lang == Language::Rust && !rust_crate_map.is_empty() {
                resolve_rust_import(
                    &rust_crate_map,
                    graph,
                    root,
                    *import_id,
                    module_name,
                    file_path,
                    &mut edges_to_add,
                );
            }
            continue;
        }

        // Determine if this is a `from X import Y` or plain `import X`
        let is_from_import = sig.starts_with("from ");

        if is_from_import {
            // Resolve the module part
            let resolved_module = if module_name.starts_with('.') {
                resolve_relative_import(file_path, root, module_name)
            } else {
                Some(module_name.clone())
            };

            if let Some(mod_path) = resolved_module {
                // Try to find the target file
                if let Some(&target_file_id) = module_map.get(&mod_path) {
                    // Get the imported items from the edge metadata
                    let items = get_import_items(graph, *import_id);

                    if items.is_empty() {
                        // from X import * or unknown
                        let edge = GirEdge::new(EdgeKind::Imports)
                            .with_confidence(0.7);
                        edges_to_add.push((*import_id, target_file_id, edge));
                    } else {
                        for item in &items {
                            // Try to resolve to a specific symbol
                            if let Some(&sym_id) =
                                symbol_map.get(&(mod_path.clone(), item.clone()))
                            {
                                let edge = GirEdge::new(EdgeKind::ImportsFrom)
                                    .with_confidence(1.0)
                                    .with_metadata(EdgeMetadata::Import {
                                        alias: None,
                                        items: vec![item.clone()],
                                    });
                                edges_to_add.push((*import_id, sym_id, edge));
                            } else {
                                // Can't resolve symbol, link to file with lower confidence
                                let edge = GirEdge::new(EdgeKind::ImportsFrom)
                                    .with_confidence(0.7)
                                    .with_metadata(EdgeMetadata::Import {
                                        alias: None,
                                        items: vec![item.clone()],
                                    });
                                edges_to_add.push((*import_id, target_file_id, edge));
                            }
                        }
                    }
                } else {
                    // Module not found in project -- likely third-party, mark heuristic
                    debug!(
                        "Could not resolve module '{}' for import in {}",
                        mod_path,
                        file_path.display()
                    );
                }
            }
        } else {
            // Plain `import X` -- resolve to file
            let resolved = if module_name.starts_with('.') {
                resolve_relative_import(file_path, root, module_name)
            } else {
                // Extract the module name from `import X.Y.Z`
                let name = sig
                    .strip_prefix("import ")
                    .unwrap_or(module_name)
                    .split(" as ")
                    .next()
                    .unwrap_or(module_name)
                    .trim();
                Some(name.to_string())
            };

            if let Some(mod_path) = resolved {
                if let Some(&target_file_id) = module_map.get(&mod_path) {
                    let edge = GirEdge::new(EdgeKind::Imports)
                        .with_confidence(1.0);
                    edges_to_add.push((*import_id, target_file_id, edge));
                } else {
                    // Try prefix match for submodule imports like `import os.path`
                    let first = mod_path.split('.').next().unwrap_or(&mod_path);
                    if let Some(&target_file_id) = module_map.get(first) {
                        let edge = GirEdge::new(EdgeKind::Imports)
                            .with_confidence(0.3);
                        edges_to_add.push((*import_id, target_file_id, edge));
                    }
                }
            }
        }
    }

    let added = edges_to_add.len();
    for (src, tgt, edge) in edges_to_add {
        graph.add_edge(src, tgt, edge);
    }

    debug!("Phase 4 (Import Resolution): added {} edges", added);
}

/// Resolve a Rust `use` import path against the workspace crate map.
/// Handles: `use graphy_core::CodeGraph`, `use crate::module::item`,
/// `use super::sibling`, `use self::child`.
fn resolve_rust_import(
    crate_map: &HashMap<String, PathBuf>,
    graph: &CodeGraph,
    _root: &Path,
    import_id: SymbolId,
    module_name: &str,
    importing_file: &Path,
    edges: &mut Vec<(SymbolId, SymbolId, GirEdge)>,
) {
    // Split the use path on `::`
    let parts: Vec<&str> = module_name.split("::").collect();
    if parts.is_empty() {
        return;
    }

    let first = parts[0];
    let symbol_name = parts.last().copied().unwrap_or("");
    if symbol_name.is_empty() || symbol_name == "*" {
        return;
    }

    // Determine the crate source directory
    let src_dir = if first == "crate" || first == "self" || first == "super" {
        // Relative import — resolve within the importing file's crate
        find_crate_src_for_file(crate_map, importing_file)
    } else {
        // External crate import
        crate_map.get(first).cloned()
    };

    let Some(src_dir) = src_dir else {
        trace!("Rust import: crate '{}' not in workspace", first);
        return;
    };

    // Look up the symbol by name in files under the resolved crate's src directory
    let target = graph
        .all_nodes()
        .find(|n| {
            n.name == symbol_name
                && n.file_path.starts_with(&src_dir)
                && matches!(
                    n.kind,
                    NodeKind::Function
                        | NodeKind::Method
                        | NodeKind::Class
                        | NodeKind::Struct
                        | NodeKind::Enum
                        | NodeKind::Trait
                        | NodeKind::Interface
                        | NodeKind::Constant
                        | NodeKind::TypeAlias
                        | NodeKind::Variable
                )
        })
        .map(|n| n.id);

    if let Some(target_id) = target {
        let edge = GirEdge::new(EdgeKind::ImportsFrom)
            .with_confidence(1.0)
            .with_metadata(EdgeMetadata::Import {
                alias: None,
                items: vec![symbol_name.to_string()],
            });
        edges.push((import_id, target_id, edge));
        debug!("Rust cross-crate: {} -> {} (resolved in {:?})", module_name, symbol_name, src_dir);
    }
}

/// Find the `src/` directory of the crate that contains the given file.
fn find_crate_src_for_file(
    crate_map: &HashMap<String, PathBuf>,
    file_path: &Path,
) -> Option<PathBuf> {
    // Find the crate whose src_dir is a prefix of the file path
    crate_map
        .values()
        .find(|src_dir| file_path.starts_with(src_dir))
        .cloned()
}

/// Extract imported item names from the ImportsFrom edges pointing from a parent to this import.
fn get_import_items(graph: &CodeGraph, import_id: SymbolId) -> Vec<String> {
    let Some(idx) = graph.get_node_index(import_id) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    for edge in graph.graph.edges_directed(idx, Direction::Incoming) {
        if let EdgeMetadata::Import {
            items: ref edge_items,
            ..
        } = edge.weight().metadata
        {
            items.extend(edge_items.iter().cloned());
        }
    }

    items
}

/// Resolve a JS/TS import path to a relative module path from root.
/// e.g., `./utils` from `src/components/App.tsx` -> `src/components/utils`
fn resolve_js_import(importing_file: &Path, root: &Path, module_name: &str) -> Option<String> {
    // Skip node_modules / bare specifiers (no ./ or ../ prefix)
    if !module_name.starts_with('.') {
        return None;
    }

    let dir = importing_file.parent()?;
    let resolved = dir.join(module_name);

    // Normalize the path and make it relative to root
    // Use components to resolve .. without requiring the path to exist
    let mut parts: Vec<std::path::Component> = Vec::new();
    for component in resolved.components() {
        match component {
            std::path::Component::ParentDir => { parts.pop(); }
            std::path::Component::CurDir => {}
            other => parts.push(other),
        }
    }
    let normalized: PathBuf = parts.iter().collect();
    let rel = normalized.strip_prefix(root).ok()?;

    Some(rel.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_python_module() {
        assert_eq!(path_to_python_module(Path::new("utils/math.py")), "utils.math");
        assert_eq!(path_to_python_module(Path::new("utils/__init__.py")), "utils");
        assert_eq!(path_to_python_module(Path::new("main.py")), "main");
    }

    #[test]
    fn test_resolve_relative() {
        let file = Path::new("/project/pkg/sub/mod.py");
        let root = Path::new("/project");

        assert_eq!(
            resolve_relative_import(file, root, ".sibling"),
            Some("pkg.sub.sibling".to_string())
        );
        assert_eq!(
            resolve_relative_import(file, root, "..parent"),
            Some("pkg.parent".to_string())
        );
    }

    #[test]
    fn test_path_to_python_module_nested() {
        assert_eq!(path_to_python_module(Path::new("a/b/c/d.py")), "a.b.c.d");
    }

    #[test]
    fn test_resolve_relative_root_file() {
        let file = Path::new("/project/mod.py");
        let root = Path::new("/project");
        // Single dot import from root — goes to parent which is project itself
        let result = resolve_relative_import(file, root, ".sibling");
        assert_eq!(result, Some("sibling".to_string()));
    }

    #[test]
    fn test_resolve_js_import_relative() {
        let file = Path::new("/proj/src/components/App.tsx");
        let root = Path::new("/proj");
        let result = resolve_js_import(file, root, "./utils");
        assert_eq!(result, Some("src/components/utils".to_string()));
    }

    #[test]
    fn test_resolve_js_import_bare_specifier() {
        let file = Path::new("/proj/src/App.tsx");
        let root = Path::new("/proj");
        // Bare specifiers (no ./) should return None
        let result = resolve_js_import(file, root, "react");
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_js_import_parent() {
        let file = Path::new("/proj/src/components/sub/App.tsx");
        let root = Path::new("/proj");
        let result = resolve_js_import(file, root, "../utils");
        assert_eq!(result, Some("src/components/utils".to_string()));
    }

    #[test]
    fn test_resolve_imports_empty_graph() {
        let mut graph = graphy_core::CodeGraph::new();
        let root = Path::new("/tmp/empty");
        // Should not panic
        resolve_imports(&mut graph, root);
    }
}
