//! Shared detection helpers for framework plugins.

use std::path::Path;

use graphy_core::{CodeGraph, NodeKind};

pub fn has_npm_dep(root: &Path, name: &str) -> bool {
    let pkg = root.join("package.json");
    std::fs::read_to_string(pkg)
        .map(|c| c.contains(&format!("\"{}\"", name)))
        .unwrap_or(false)
}

pub fn has_composer_dep(root: &Path, name: &str) -> bool {
    let pkg = root.join("composer.json");
    std::fs::read_to_string(pkg)
        .map(|c| c.contains(&format!("\"{}\"", name)))
        .unwrap_or(false)
}

pub fn has_pip_dep(root: &Path, name: &str) -> bool {
    for f in &[
        "requirements.txt",
        "pyproject.toml",
        "Pipfile",
        "setup.py",
        "setup.cfg",
    ] {
        if let Ok(content) = std::fs::read_to_string(root.join(f)) {
            if content.contains(name) {
                return true;
            }
        }
    }
    false
}

pub fn has_cargo_dep(root: &Path, name: &str) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .map(|c| c.contains(name))
        .unwrap_or(false)
}

pub fn has_gem_dep(root: &Path, name: &str) -> bool {
    std::fs::read_to_string(root.join("Gemfile"))
        .map(|c| c.contains(name))
        .unwrap_or(false)
}

pub fn has_gradle_or_maven_dep(root: &Path, name: &str) -> bool {
    for f in &["build.gradle", "build.gradle.kts", "pom.xml"] {
        if let Ok(content) = std::fs::read_to_string(root.join(f)) {
            if content.contains(name) {
                return true;
            }
        }
    }
    false
}

pub fn has_import_of(graph: &CodeGraph, module: &str) -> bool {
    graph
        .all_nodes()
        .any(|n| n.kind == NodeKind::Import && n.name.contains(module))
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, Language, Span};
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn has_npm_dep_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"express": "^4.18.0"}}"#,
        )
        .unwrap();
        assert!(has_npm_dep(tmp.path(), "express"));
        assert!(!has_npm_dep(tmp.path(), "react"));
    }

    #[test]
    fn has_npm_dep_missing_file() {
        let tmp = TempDir::new().unwrap();
        assert!(!has_npm_dep(tmp.path(), "express"));
    }

    #[test]
    fn has_cargo_dep_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\nserde = \"1\"\ntokio = { version = \"1\" }\n",
        )
        .unwrap();
        assert!(has_cargo_dep(tmp.path(), "serde"));
        assert!(has_cargo_dep(tmp.path(), "tokio"));
        assert!(!has_cargo_dep(tmp.path(), "actix"));
    }

    #[test]
    fn has_pip_dep_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("requirements.txt"), "flask==2.3.0\nrequests\n").unwrap();
        assert!(has_pip_dep(tmp.path(), "flask"));
        assert!(has_pip_dep(tmp.path(), "requests"));
        assert!(!has_pip_dep(tmp.path(), "django"));
    }

    #[test]
    fn has_gem_dep_found() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Gemfile"), "gem 'rails'\ngem 'puma'\n").unwrap();
        assert!(has_gem_dep(tmp.path(), "rails"));
        assert!(!has_gem_dep(tmp.path(), "sinatra"));
    }

    #[test]
    fn has_import_of_found() {
        let mut g = CodeGraph::new();
        let import = GirNode::new(
            "flask".to_string(),
            NodeKind::Import,
            PathBuf::from("app.py"),
            Span::new(1, 0, 1, 20),
            Language::Python,
        );
        g.add_node(import);
        assert!(has_import_of(&g, "flask"));
        assert!(!has_import_of(&g, "django"));
    }

    #[test]
    fn has_import_of_partial_match() {
        let mut g = CodeGraph::new();
        let import = GirNode::new(
            "flask.blueprints".to_string(),
            NodeKind::Import,
            PathBuf::from("app.py"),
            Span::new(1, 0, 1, 30),
            Language::Python,
        );
        g.add_node(import);
        assert!(has_import_of(&g, "flask"));
    }

    #[test]
    fn has_import_of_empty_graph() {
        let g = CodeGraph::new();
        assert!(!has_import_of(&g, "anything"));
    }
}
