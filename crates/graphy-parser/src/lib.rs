pub mod frontend;
pub mod helpers;
pub mod python;
pub mod rust_lang;
pub mod svelte;
pub mod tags_frontend;
pub mod tags_registry;
pub mod typescript;

pub mod dynamic_loader;
pub mod grammar_compiler;

use std::path::Path;

use anyhow::Result;
use graphy_core::{Language, ParseOutput};

use crate::frontend::LanguageFrontend;
use crate::python::PythonFrontend;
use crate::rust_lang::RustFrontend;
use crate::svelte::SvelteFrontend;
use crate::tags_frontend::TagsFrontend;
use crate::typescript::TypeScriptFrontend;

/// Parse a single file, auto-detecting the language from its extension.
pub fn parse_file(path: &Path, source: &str) -> Result<ParseOutput> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let lang = Language::from_extension(ext)
        .ok_or_else(|| anyhow::anyhow!("Unsupported file extension: {ext}"))?;

    match lang {
        // Built-in: custom frontends for deep analysis
        Language::Python => PythonFrontend::new().parse(path, source),
        Language::TypeScript | Language::JavaScript => TypeScriptFrontend::new().parse(path, source),
        Language::Rust => RustFrontend::new().parse(path, source),
        Language::Svelte => SvelteFrontend::new().parse(path, source),
        // Dynamic: loaded from ~/.config/graphy/grammars/
        other => {
            if let Some(config) = tags_registry::tags_config_for_language(other) {
                TagsFrontend::new(config).parse(path, source)
            } else {
                let hint = dynamic_loader::grammar_info_for_language(other)
                    .map(|info| format!("Install with: graphy lang add {}", info.name))
                    .unwrap_or_else(|| "No grammar available for this language".to_string());
                Err(anyhow::anyhow!("Language {other:?} grammar not installed. {hint}"))
            }
        }
    }
}

/// Parse multiple files in parallel using rayon.
pub fn parse_files(files: &[(std::path::PathBuf, String)]) -> Vec<(std::path::PathBuf, Result<ParseOutput>)> {
    use rayon::prelude::*;

    files
        .par_iter()
        .map(|(path, source)| {
            let result = parse_file(path, source);
            (path.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_file_python() {
        let path = PathBuf::from("test.py");
        let source = "def hello():\n    pass\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_typescript() {
        let path = PathBuf::from("test.ts");
        let source = "function greet(): void {}\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_javascript() {
        let path = PathBuf::from("test.js");
        let source = "function add(a, b) { return a + b; }\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_rust() {
        let path = PathBuf::from("test.rs");
        let source = "fn main() {}\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_svelte() {
        let path = PathBuf::from("Component.svelte");
        let source = "<script>\nlet x = 1;\n</script>\n<p>Hello</p>";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_unsupported_extension() {
        let path = PathBuf::from("test.xyz");
        let source = "irrelevant";
        let result = parse_file(&path, source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Unsupported") || err_msg.contains("xyz"));
    }

    #[test]
    fn parse_file_no_extension() {
        let path = PathBuf::from("Makefile");
        let source = "all: build\n";
        let result = parse_file(&path, source);
        assert!(result.is_err());
    }

    #[test]
    fn parse_files_parallel() {
        let files = vec![
            (PathBuf::from("a.py"), "def a(): pass\n".to_string()),
            (PathBuf::from("b.ts"), "function b() {}\n".to_string()),
            (PathBuf::from("c.rs"), "fn c() {}\n".to_string()),
        ];
        let results = parse_files(&files);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, r)| r.is_ok()));
    }

    #[test]
    fn parse_files_empty() {
        let files: Vec<(PathBuf, String)> = vec![];
        let results = parse_files(&files);
        assert!(results.is_empty());
    }

    #[test]
    fn parse_files_mixed_success_failure() {
        let files = vec![
            (PathBuf::from("good.py"), "def ok(): pass\n".to_string()),
            (PathBuf::from("bad.xyz"), "???".to_string()),
        ];
        let results = parse_files(&files);
        assert_eq!(results.len(), 2);
        assert!(results[0].1.is_ok());
        assert!(results[1].1.is_err());
    }

    #[test]
    fn parse_file_jsx_extension() {
        let path = PathBuf::from("App.jsx");
        let source = "function App() { return <div/>; }\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_file_tsx_extension() {
        let path = PathBuf::from("App.tsx");
        let source = "function App(): JSX.Element { return <div/>; }\n";
        let result = parse_file(&path, source);
        assert!(result.is_ok());
    }
}
