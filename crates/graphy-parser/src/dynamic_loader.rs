//! Dynamic tree-sitter grammar loading.
//!
//! Loads compiled tree-sitter grammars (.so/.dylib) from
//! `~/.config/graphy/grammars/<lang>/` at runtime, similar
//! to how Neovim loads grammars via `:TSInstall`.

use std::borrow::Cow;
use std::path::PathBuf;

use graphy_core::Language;
use tracing::{debug, warn};

use crate::tags_registry::TagsLanguageConfig;

#[cfg(target_os = "macos")]
pub const LIB_EXT: &str = "dylib";
#[cfg(target_os = "linux")]
pub const LIB_EXT: &str = "so";
#[cfg(target_os = "windows")]
pub const LIB_EXT: &str = "dll";

/// Metadata for a known tree-sitter grammar.
pub struct GrammarInfo {
    pub name: &'static str,
    pub language: Language,
    pub repo_url: &'static str,
    pub ts_func_symbol: &'static str,
    pub extensions: &'static [&'static str],
    pub src_dir: &'static str,
    pub has_cpp_scanner: bool,
    /// Git ref (tag/commit) compatible with tree-sitter ABI 14 (tree-sitter 0.24).
    /// Use `None` to clone HEAD (risky if the grammar moves to a newer ABI).
    pub compatible_ref: Option<&'static str>,
}

// Pinned to commits compatible with tree-sitter 0.24 (ABI 14).
pub const KNOWN_GRAMMARS: &[GrammarInfo] = &[
    GrammarInfo {
        name: "go",
        language: Language::Go,
        repo_url: "https://github.com/tree-sitter/tree-sitter-go",
        ts_func_symbol: "tree_sitter_go",
        extensions: &["go"],
        src_dir: "src",
        has_cpp_scanner: false,
        compatible_ref: Some("3c3775faa968158a8b4ac190a7fda867fd5fb748"),
    },
    GrammarInfo {
        name: "java",
        language: Language::Java,
        repo_url: "https://github.com/tree-sitter/tree-sitter-java",
        ts_func_symbol: "tree_sitter_java",
        extensions: &["java"],
        src_dir: "src",
        has_cpp_scanner: false,
        compatible_ref: Some("94703d5a6bed02b98e438d7cad1136c01a60ba2c"),
    },
    GrammarInfo {
        name: "php",
        language: Language::Php,
        repo_url: "https://github.com/tree-sitter/tree-sitter-php",
        ts_func_symbol: "tree_sitter_php",
        extensions: &["php"],
        src_dir: "php/src",
        has_cpp_scanner: false,
        compatible_ref: Some("43aad2b9a98aa8e603ea0cf5bb630728a5591ad8"),
    },
    GrammarInfo {
        name: "c",
        language: Language::C,
        repo_url: "https://github.com/tree-sitter/tree-sitter-c",
        ts_func_symbol: "tree_sitter_c",
        extensions: &["c", "h"],
        src_dir: "src",
        has_cpp_scanner: false,
        compatible_ref: Some("362a8a41b265056592a0c3771664a21d23a71392"),
    },
    GrammarInfo {
        name: "cpp",
        language: Language::Cpp,
        repo_url: "https://github.com/tree-sitter/tree-sitter-cpp",
        ts_func_symbol: "tree_sitter_cpp",
        extensions: &["cpp", "cc", "cxx", "hpp"],
        src_dir: "src",
        has_cpp_scanner: true,
        compatible_ref: Some("f41e1a044c8a84ea9fa8577fdd2eab92ec96de02"),
    },
    GrammarInfo {
        name: "c-sharp",
        language: Language::CSharp,
        repo_url: "https://github.com/tree-sitter/tree-sitter-c-sharp",
        ts_func_symbol: "tree_sitter_c_sharp",
        extensions: &["cs"],
        src_dir: "src",
        has_cpp_scanner: true,
        compatible_ref: Some("362a8a41b265056592a0c3771664a21d23a71392"),
    },
    GrammarInfo {
        name: "ruby",
        language: Language::Ruby,
        repo_url: "https://github.com/tree-sitter/tree-sitter-ruby",
        ts_func_symbol: "tree_sitter_ruby",
        extensions: &["rb"],
        src_dir: "src",
        has_cpp_scanner: true,
        compatible_ref: Some("71bd32fb7607035768799732addba884a37a6210"),
    },
    GrammarInfo {
        name: "kotlin",
        language: Language::Kotlin,
        repo_url: "https://github.com/fwcd/tree-sitter-kotlin",
        ts_func_symbol: "tree_sitter_kotlin",
        extensions: &["kt", "kts"],
        src_dir: "src",
        has_cpp_scanner: true,
        compatible_ref: None,
    },
];

/// Base directory for dynamic grammars: `~/.config/graphy/grammars/`
pub fn grammars_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        })
        .join("graphy")
        .join("grammars")
}

/// Directory for a specific language grammar.
pub fn grammar_dir_for(name: &str) -> PathBuf {
    grammars_dir().join(name)
}

/// Check if a dynamic grammar is installed.
pub fn is_installed(name: &str) -> bool {
    grammar_dir_for(name)
        .join(format!("parser.{LIB_EXT}"))
        .exists()
}

/// List all installed dynamic grammars.
pub fn list_installed() -> Vec<String> {
    let dir = grammars_dir();
    if !dir.is_dir() {
        return vec![];
    }
    let mut installed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join(format!("parser.{LIB_EXT}")).exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    installed.push(name.to_string());
                }
            }
        }
    }
    installed.sort();
    installed
}

/// Look up a known grammar by Language enum.
pub fn grammar_info_for_language(lang: Language) -> Option<&'static GrammarInfo> {
    KNOWN_GRAMMARS.iter().find(|g| g.language == lang)
}

/// Look up a known grammar by name.
pub fn grammar_info_by_name(name: &str) -> Option<&'static GrammarInfo> {
    KNOWN_GRAMMARS.iter().find(|g| g.name == name)
}

/// Load a dynamic grammar at runtime.
///
/// Looks for `~/.config/graphy/grammars/<name>/parser.{so,dylib}` and
/// loads it via `libloading`. The shared library must export a
/// `tree_sitter_<name>()` function returning a tree_sitter Language.
pub fn load_dynamic_grammar(lang: Language) -> Option<TagsLanguageConfig> {
    let info = grammar_info_for_language(lang)?;
    let dir = grammar_dir_for(info.name);
    let lib_path = dir.join(format!("parser.{LIB_EXT}"));

    if !lib_path.exists() {
        debug!("No dynamic grammar for {:?} at {}", lang, lib_path.display());
        return None;
    }

    // Load the shared library.
    // SAFETY: We trust the .so was compiled from a tree-sitter grammar repo.
    let lib = match unsafe { libloading::Library::new(&lib_path) } {
        Ok(lib) => lib,
        Err(e) => {
            warn!("Failed to load grammar library {}: {e}", lib_path.display());
            return None;
        }
    };

    let ts_language = unsafe {
        let func: libloading::Symbol<unsafe extern "C" fn() -> tree_sitter::Language> =
            match lib.get(info.ts_func_symbol.as_bytes()) {
                Ok(f) => f,
                Err(e) => {
                    warn!(
                        "Symbol '{}' not found in {}: {e}",
                        info.ts_func_symbol,
                        lib_path.display()
                    );
                    return None;
                }
            };
        func()
    };

    // Leak the library handle so it stays loaded for the process lifetime.
    // Dropping it would invalidate the Language's function pointers.
    std::mem::forget(lib);

    let tags_query = load_tags_query(info.name, &dir);

    Some(TagsLanguageConfig {
        ts_language,
        tags_query: Cow::Owned(tags_query),
        language: lang,
    })
}

/// Load tags.scm: prefer user's file in grammar dir, fall back to bundled.
fn load_tags_query(name: &str, grammar_dir: &std::path::Path) -> String {
    let custom_path = grammar_dir.join("tags.scm");
    if custom_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&custom_path) {
            debug!("Using custom tags.scm for {name}");
            return content;
        }
    }
    bundled_tags_query(name)
        .unwrap_or("")
        .to_string()
}

/// Bundled tags.scm queries (embedded at compile time as fallback).
pub fn bundled_tags_query(name: &str) -> Option<&'static str> {
    match name {
        "go" => Some(include_str!("../tags/go.scm")),
        "java" => Some(include_str!("../tags/java.scm")),
        "php" => Some(include_str!("../tags/php.scm")),
        "c" => Some(include_str!("../tags/c.scm")),
        "cpp" => Some(include_str!("../tags/cpp.scm")),
        "c-sharp" => Some(include_str!("../tags/csharp.scm")),
        "ruby" => Some(include_str!("../tags/ruby.scm")),
        "kotlin" => Some(include_str!("../tags/kotlin.scm")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_grammars_has_entries() {
        assert!(KNOWN_GRAMMARS.len() >= 7);
    }

    #[test]
    fn grammar_info_by_name_works() {
        let go = grammar_info_by_name("go").unwrap();
        assert_eq!(go.language, Language::Go);
        assert_eq!(go.ts_func_symbol, "tree_sitter_go");
    }

    #[test]
    fn grammar_info_for_language_works() {
        let java = grammar_info_for_language(Language::Java).unwrap();
        assert_eq!(java.name, "java");
    }

    #[test]
    fn grammars_dir_is_valid() {
        let dir = grammars_dir();
        assert!(dir.to_string_lossy().contains("graphy"));
        assert!(dir.to_string_lossy().contains("grammars"));
    }

    #[test]
    fn bundled_tags_queries_exist() {
        for name in &["go", "java", "php", "c", "cpp", "c-sharp", "ruby"] {
            let query = bundled_tags_query(name);
            assert!(query.is_some(), "Missing bundled tags.scm for {name}");
            assert!(!query.unwrap().is_empty(), "Empty tags.scm for {name}");
        }
    }

    #[test]
    fn unknown_grammar_returns_none() {
        assert!(grammar_info_by_name("brainfuck").is_none());
        assert!(bundled_tags_query("brainfuck").is_none());
    }

    #[test]
    fn all_known_grammars_have_consistent_metadata() {
        // Every entry in KNOWN_GRAMMARS should have non-empty required fields,
        // unique names, and extensions that don't overlap across grammars.
        let mut seen_names = std::collections::HashSet::new();
        let mut seen_extensions = std::collections::HashMap::new();

        for info in KNOWN_GRAMMARS {
            // Name must be non-empty and unique
            assert!(!info.name.is_empty(), "Grammar name is empty");
            assert!(seen_names.insert(info.name), "Duplicate grammar name: {}", info.name);

            // Must have at least one file extension
            assert!(!info.extensions.is_empty(), "Grammar {} has no extensions", info.name);

            // Extensions should not overlap with other grammars
            for ext in info.extensions {
                if let Some(prev) = seen_extensions.insert(*ext, info.name) {
                    panic!("Extension '.{ext}' claimed by both '{prev}' and '{}'", info.name);
                }
            }

            // ts_func_symbol should start with "tree_sitter_"
            assert!(
                info.ts_func_symbol.starts_with("tree_sitter_"),
                "Grammar {} has unexpected symbol: {}",
                info.name, info.ts_func_symbol
            );

            // repo_url must be a valid-looking URL
            assert!(
                info.repo_url.starts_with("https://"),
                "Grammar {} has invalid repo URL: {}",
                info.name, info.repo_url
            );

            // Lookup by name and by language should be consistent
            let by_name = grammar_info_by_name(info.name).unwrap();
            assert_eq!(by_name.language, info.language);
            let by_lang = grammar_info_for_language(info.language).unwrap();
            assert_eq!(by_lang.name, info.name);
        }
    }
}
