use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;
use rayon::prelude::*;

use graphy_core::Language;

/// A discovered source file with its metadata.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub language: Language,
    pub content_hash: u64,
}

/// Phase 1: Walk the directory tree respecting .gitignore, filter by supported languages.
pub fn discover_files(root: &Path) -> Result<Vec<DiscoveredFile>> {
    let entries: Vec<PathBuf> = WalkBuilder::new(root)
        .hidden(true) // respect hidden files
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file()))
        .map(|entry| entry.into_path())
        .collect();

    let files: Vec<DiscoveredFile> = entries
        .par_iter()
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?;
            let language = Language::from_extension(ext)?;

            // Quick content hash for incremental indexing
            let content = std::fs::read(path).ok()?;
            let hash = simple_hash(&content);

            Some(DiscoveredFile {
                path: path.clone(),
                language,
                content_hash: hash,
            })
        })
        .collect();

    Ok(files)
}

/// Load file content hash cache from a previous run.
pub fn load_hash_cache(cache_path: &Path) -> HashMap<PathBuf, u64> {
    std::fs::read_to_string(cache_path)
        .ok()
        .map(|content| {
            content
                .lines()
                .filter_map(|line| {
                    let (path, hash) = line.split_once('\t')?;
                    let hash = hash.parse().ok()?;
                    Some((PathBuf::from(path), hash))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Save file content hashes for incremental indexing.
pub fn save_hash_cache(cache_path: &Path, files: &[DiscoveredFile]) -> Result<()> {
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content: String = files
        .iter()
        .map(|f| format!("{}\t{}", f.path.display(), f.content_hash))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(cache_path, content)?;
    Ok(())
}

/// FNV-1a hash for quick content hashing.
fn simple_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn simple_hash_empty() {
        let hash = simple_hash(b"");
        assert_eq!(hash, 0xcbf29ce484222325); // FNV-1a offset basis
    }

    #[test]
    fn simple_hash_deterministic() {
        let h1 = simple_hash(b"hello world");
        let h2 = simple_hash(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn simple_hash_different_content() {
        let h1 = simple_hash(b"hello");
        let h2 = simple_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn discover_files_filters_by_language() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("main.py"), "print('hello')").unwrap();
        fs::write(tmp.path().join("readme.md"), "# Hello").unwrap();
        fs::write(tmp.path().join("data.txt"), "some data").unwrap();

        let files = discover_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].language, Language::Python);
    }

    #[test]
    fn discover_files_multiple_languages() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("app.py"), "pass").unwrap();
        fs::write(tmp.path().join("lib.rs"), "fn main() {}").unwrap();
        fs::write(tmp.path().join("index.ts"), "export {}").unwrap();

        let files = discover_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 3);
        let languages: Vec<_> = files.iter().map(|f| f.language).collect();
        assert!(languages.contains(&Language::Python));
        assert!(languages.contains(&Language::Rust));
        assert!(languages.contains(&Language::TypeScript));
    }

    #[test]
    fn discover_files_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let files = discover_files(tmp.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn hash_cache_round_trip() {
        let tmp = TempDir::new().unwrap();
        let cache_path = tmp.path().join("cache.txt");

        let files = vec![
            DiscoveredFile {
                path: PathBuf::from("/project/main.py"),
                language: Language::Python,
                content_hash: 12345,
            },
            DiscoveredFile {
                path: PathBuf::from("/project/lib.rs"),
                language: Language::Rust,
                content_hash: 67890,
            },
        ];

        save_hash_cache(&cache_path, &files).unwrap();
        let loaded = load_hash_cache(&cache_path);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[&PathBuf::from("/project/main.py")], 12345);
        assert_eq!(loaded[&PathBuf::from("/project/lib.rs")], 67890);
    }

    #[test]
    fn load_hash_cache_missing_file() {
        let cache = load_hash_cache(Path::new("/nonexistent/cache.txt"));
        assert!(cache.is_empty());
    }

    #[test]
    fn load_hash_cache_malformed_lines() {
        let tmp = TempDir::new().unwrap();
        let cache_path = tmp.path().join("cache.txt");
        fs::write(&cache_path, "valid/path\t12345\ninvalid_line\n\tno_path").unwrap();

        let cache = load_hash_cache(&cache_path);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[&PathBuf::from("valid/path")], 12345);
    }
}
