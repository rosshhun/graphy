use std::collections::HashSet;
use std::path::Path;

use graphy_core::{
    CodeGraph, EdgeKind, GirEdge, GirNode, Language, NodeKind, Span, SymbolId, Visibility,
};

/// Phase 2: Build File and Folder hierarchy nodes in the graph.
pub fn build_structure(graph: &mut CodeGraph, root: &Path, files: &[super::discovery::DiscoveredFile]) {
    let mut created_folders: HashSet<std::path::PathBuf> = HashSet::new();

    // Create root folder node
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".to_string());

    let root_node = GirNode {
        id: SymbolId::new(root, &root_name, NodeKind::Folder, 0),
        name: root_name,
        kind: NodeKind::Folder,
        file_path: root.to_path_buf(),
        span: Span::new(0, 0, 0, 0),
        visibility: Visibility::Public,
        language: Language::Python, // doesn't matter for folders
        signature: None,
        complexity: None,
        confidence: 1.0,
        doc: None,
        coverage: None,
    };
    let root_id = root_node.id;
    graph.add_node(root_node);
    created_folders.insert(root.to_path_buf());

    for file in files {
        // Create intermediate folder nodes
        if let Ok(relative) = file.path.strip_prefix(root) {
            let mut current_path = root.to_path_buf();
            let mut parent_id = root_id;

            // Create folder nodes for each path component (except the file itself)
            let components: Vec<_> = relative.parent().map(|p| p.components().collect()).unwrap_or_default();
            for component in components {
                current_path.push(component);

                if !created_folders.contains(&current_path) {
                    let folder_name = component.as_os_str().to_string_lossy().into_owned();
                    let folder_node = GirNode {
                        id: SymbolId::new(&current_path, &folder_name, NodeKind::Folder, 0),
                        name: folder_name,
                        kind: NodeKind::Folder,
                        file_path: current_path.clone(),
                        span: Span::new(0, 0, 0, 0),
                        visibility: Visibility::Public,
                        language: file.language,
                        signature: None,
                        complexity: None,
                        confidence: 1.0,
                        doc: None,
                        coverage: None,
                    };
                    let folder_id = folder_node.id;
                    graph.add_node(folder_node);
                    graph.add_edge(parent_id, folder_id, GirEdge::new(EdgeKind::Contains));
                    created_folders.insert(current_path.clone());
                    parent_id = SymbolId::new(
                        &current_path,
                        &current_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        NodeKind::Folder,
                        0,
                    );
                } else {
                    parent_id = SymbolId::new(
                        &current_path,
                        &current_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        NodeKind::Folder,
                        0,
                    );
                }
            }

            // The file node will be created by the parser (Phase 3).
            // We just need to connect it to its parent folder later.
            // For now, store the relationship info.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::DiscoveredFile;
    use std::path::PathBuf;

    #[test]
    fn build_structure_empty() {
        let mut g = CodeGraph::new();
        build_structure(&mut g, Path::new("/project"), &[]);
        // Should have just the root folder node
        assert_eq!(g.find_by_kind(NodeKind::Folder).len(), 1);
    }

    #[test]
    fn build_structure_flat_files() {
        let mut g = CodeGraph::new();
        let files = vec![
            DiscoveredFile {
                path: PathBuf::from("/project/main.py"),
                language: Language::Python,
                content_hash: 123,
            },
            DiscoveredFile {
                path: PathBuf::from("/project/utils.py"),
                language: Language::Python,
                content_hash: 456,
            },
        ];
        build_structure(&mut g, Path::new("/project"), &files);
        // Root folder only — no intermediate folders for flat structure
        let folders = g.find_by_kind(NodeKind::Folder);
        assert_eq!(folders.len(), 1);
    }

    #[test]
    fn build_structure_nested_directories() {
        let mut g = CodeGraph::new();
        let files = vec![
            DiscoveredFile {
                path: PathBuf::from("/project/src/lib.rs"),
                language: Language::Rust,
                content_hash: 100,
            },
            DiscoveredFile {
                path: PathBuf::from("/project/src/utils/helpers.rs"),
                language: Language::Rust,
                content_hash: 200,
            },
        ];
        build_structure(&mut g, Path::new("/project"), &files);
        // Should have: root folder + src + utils = 3 folders
        let folders = g.find_by_kind(NodeKind::Folder);
        assert!(folders.len() >= 3);
    }

    #[test]
    fn build_structure_deduplicates_folders() {
        let mut g = CodeGraph::new();
        let files = vec![
            DiscoveredFile {
                path: PathBuf::from("/project/src/a.py"),
                language: Language::Python,
                content_hash: 1,
            },
            DiscoveredFile {
                path: PathBuf::from("/project/src/b.py"),
                language: Language::Python,
                content_hash: 2,
            },
        ];
        build_structure(&mut g, Path::new("/project"), &files);
        // Only one "src" folder despite two files in it
        let src_folders: Vec<_> = g.find_by_kind(NodeKind::Folder)
            .into_iter()
            .filter(|f| f.name == "src")
            .collect();
        assert_eq!(src_folders.len(), 1);
    }
}
