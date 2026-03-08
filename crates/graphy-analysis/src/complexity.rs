//! Phase 10: Complexity metrics computation.
//!
//! Reads source files from disk and computes per-function/method metrics:
//! - Cyclomatic complexity (decision points)
//! - Cognitive complexity (nested control flow weighted higher)
//! - LOC and SLOC (non-blank, non-comment lines)
//! - Parameter count and max nesting depth

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphy_core::{CodeGraph, ComplexityMetrics, Language, NodeKind};
use tracing::debug;

/// Get decision-point keywords for cyclomatic complexity by language.
fn decision_keywords(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &[
            "if", "elif", "for", "while", "except", "with", "assert", "and", "or",
        ],
        Language::Rust => &[
            "if", "else", "for", "while", "loop", "match", "&&", "||", "?",
        ],
        Language::Go => &[
            "if", "else", "for", "switch", "select", "case", "&&", "||",
        ],
        Language::Java | Language::Kotlin | Language::CSharp => &[
            "if", "else", "for", "while", "do", "switch", "case", "catch", "&&", "||",
        ],
        Language::Ruby => &[
            "if", "elsif", "unless", "for", "while", "until", "when", "rescue", "and", "or",
        ],
        Language::Php => &[
            "if", "elseif", "for", "foreach", "while", "do", "switch", "case", "catch", "&&", "||",
        ],
        // JS/TS/Svelte/C/C++ share C-family patterns
        _ => &[
            "if", "else", "for", "while", "do", "switch", "case", "catch", "&&", "||",
        ],
    }
}

/// Get nesting keywords for cognitive complexity by language.
fn nesting_keywords(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &[
            "if", "elif", "else", "for", "while", "try", "except", "with",
        ],
        Language::Rust => &[
            "if", "else", "for", "while", "loop", "match", "unsafe",
        ],
        Language::Go => &[
            "if", "else", "for", "switch", "select",
        ],
        Language::Ruby => &[
            "if", "elsif", "else", "unless", "for", "while", "until", "begin", "rescue",
        ],
        _ => &[
            "if", "else", "for", "while", "do", "switch", "try", "catch",
        ],
    }
}

/// Phase 10: Compute complexity metrics for all functions and methods.
pub fn compute_complexity(graph: &mut CodeGraph, _root: &Path) {
    compute_complexity_filtered(graph, None);
}

/// Compute complexity metrics, optionally limited to functions in the given files.
/// When `only_files` is `None`, computes for all functions (full pipeline).
/// When `Some`, only computes for functions whose file_path is in the set.
pub fn compute_complexity_filtered(graph: &mut CodeGraph, only_files: Option<&[PathBuf]>) {
    compute_complexity_with_cache(graph, only_files, None);
}

/// Compute complexity metrics with an optional pre-loaded file cache.
/// Use this from watch mode to avoid reading files from disk while holding
/// the graph write lock.
pub fn compute_complexity_with_cache(
    graph: &mut CodeGraph,
    only_files: Option<&[PathBuf]>,
    preloaded: Option<HashMap<PathBuf, Vec<String>>>,
) {
    let mut file_cache: HashMap<PathBuf, Vec<String>> = preloaded.unwrap_or_default();

    // Collect function/method nodes with their spans
    let targets: Vec<(graphy_core::SymbolId, PathBuf, u32, u32, String, Language)> = graph
        .all_nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Constructor
            )
        })
        .filter(|n| {
            only_files.map_or(true, |files| files.contains(&n.file_path))
        })
        .map(|n| {
            (
                n.id,
                n.file_path.clone(),
                n.span.start_line,
                n.span.end_line,
                n.name.clone(),
                n.language,
            )
        })
        .collect();

    let mut updated = 0;

    for (sym_id, file_path, start_line, end_line, _name, language) in &targets {
        // Load file if not cached
        if !file_cache.contains_key(file_path) {
            if let Ok(content) = std::fs::read_to_string(file_path) {
                let lines: Vec<String> = content.lines().map(String::from).collect();
                file_cache.insert(file_path.clone(), lines);
            } else {
                continue;
            }
        }

        let Some(lines) = file_cache.get(file_path) else {
            continue;
        };

        let start = *start_line as usize;
        let end = (*end_line as usize).min(lines.len().saturating_sub(1));

        if start > end || start >= lines.len() {
            continue;
        }

        let func_lines = &lines[start..=end];
        let metrics = analyze_function_lines(func_lines, *language);

        // Count parameters from the graph
        let param_count = graph
            .children(*sym_id)
            .iter()
            .filter(|n| n.kind == NodeKind::Parameter)
            .count() as u32;

        let final_metrics = ComplexityMetrics {
            cyclomatic: metrics.cyclomatic,
            cognitive: metrics.cognitive,
            loc: metrics.loc,
            sloc: metrics.sloc,
            parameter_count: param_count,
            max_nesting_depth: metrics.max_nesting_depth,
        };

        // Update the node in the graph
        if let Some(idx) = graph.get_node_index(*sym_id) {
            if let Some(node) = graph.graph.node_weight_mut(idx) {
                node.complexity = Some(final_metrics);
                updated += 1;
            }
        }
    }

    debug!(
        "Phase 10 (Complexity): computed metrics for {}/{} functions",
        updated,
        targets.len()
    );
}

struct RawMetrics {
    cyclomatic: u32,
    cognitive: u32,
    loc: u32,
    sloc: u32,
    max_nesting_depth: u32,
}

fn analyze_function_lines(lines: &[String], lang: Language) -> RawMetrics {
    let mut cyclomatic: u32 = 1; // Base complexity
    let mut cognitive: u32 = 0;
    let mut loc: u32 = 0;
    let mut sloc: u32 = 0;
    let mut max_nesting: u32 = 0;
    let mut in_multiline_string = false;

    let decision_kw = decision_keywords(lang);
    let nesting_kw = nesting_keywords(lang);

    // Comment prefix depends on language
    let line_comment = match lang {
        Language::Python | Language::Ruby => "#",
        _ => "//",
    };

    // Track indentation of the function body
    let base_indent = lines
        .first()
        .map(|l| count_indent(l))
        .unwrap_or(0);

    for line in lines {
        loc += 1;
        let trimmed = line.trim();

        // Track multiline strings
        let triple_count = trimmed.matches("\"\"\"").count() + trimmed.matches("'''").count();
        if triple_count % 2 != 0 {
            in_multiline_string = !in_multiline_string;
        }

        if in_multiline_string {
            continue;
        }

        // Skip blank and comment-only lines for SLOC
        if trimmed.is_empty() || trimmed.starts_with(line_comment) {
            continue;
        }

        sloc += 1;

        // Compute nesting level from indentation relative to function body
        let indent = count_indent(line);
        let relative_indent = indent.saturating_sub(base_indent);
        // Assume 4-space indentation; fall back to dividing
        let nesting_level = if relative_indent > 0 {
            (relative_indent / 4).max(1)
        } else {
            0
        };

        if nesting_level > max_nesting {
            max_nesting = nesting_level;
        }
        let current_nesting = nesting_level;

        // Tokenize the line to find keywords
        let tokens = extract_keyword_tokens(trimmed);

        for token in &tokens {
            // Cyclomatic complexity: count decision points
            if decision_kw.contains(&token.as_str()) {
                cyclomatic += 1;
            }

            // Cognitive complexity: nesting keywords get weighted by nesting depth
            if nesting_kw.contains(&token.as_str()) {
                cognitive += 1 + current_nesting.saturating_sub(1);
            }

            // Boolean operators add to cognitive (language-specific ones already in decision_kw)
            if token == "and" || token == "or" || token == "&&" || token == "||" {
                cognitive += 1;
            }
        }
    }

    RawMetrics {
        cyclomatic,
        cognitive,
        loc,
        sloc,
        max_nesting_depth: max_nesting,
    }
}

/// Count leading spaces in a line.
fn count_indent(line: &str) -> u32 {
    let mut count = 0u32;
    for ch in line.chars() {
        match ch {
            ' ' => count += 1,
            '\t' => count += 4,
            _ => break,
        }
    }
    count
}

/// Extract keyword tokens from a line of code (simplistic tokenizer).
/// Avoids matching keywords inside strings or comments.
fn extract_keyword_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();

    // Strip comments
    let code = strip_comment(line);

    // Strip string literals (simplistic)
    let code = strip_strings(&code);

    // Extract word tokens
    let mut chars = code.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_alphabetic() || ch == '_' {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    word.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(word);
        } else {
            chars.next();
        }
    }

    tokens
}

/// Strip a trailing comment from a line.
fn strip_comment(line: &str) -> String {
    let mut result = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev = '\0';

    for ch in line.chars() {
        if ch == '\'' && !in_double_quote && prev != '\\' {
            in_single_quote = !in_single_quote;
        } else if ch == '"' && !in_single_quote && prev != '\\' {
            in_double_quote = !in_double_quote;
        } else if ch == '#' && !in_single_quote && !in_double_quote {
            break;
        }
        result.push(ch);
        prev = ch;
    }

    result
}

/// Replace string literals with empty strings to avoid matching keywords inside them.
fn strip_strings(code: &str) -> String {
    let mut result = String::new();
    let mut chars = code.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' || ch == '"' {
            // Skip until matching quote
            while let Some(c) = chars.next() {
                if c == ch {
                    break;
                }
                if c == '\\' {
                    chars.next(); // skip escaped char
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function() {
        let lines: Vec<String> = vec![
            "def foo(x):".to_string(),
            "    if x > 0:".to_string(),
            "        return x".to_string(),
            "    return 0".to_string(),
        ];
        let m = analyze_function_lines(&lines, Language::Python);
        assert!(m.cyclomatic >= 2); // base 1 + 1 if
        assert_eq!(m.loc, 4);
    }

    #[test]
    fn test_count_indent() {
        assert_eq!(count_indent("    hello"), 4);
        assert_eq!(count_indent("hello"), 0);
        assert_eq!(count_indent("\thello"), 4);
    }

    #[test]
    fn test_analyze_empty_lines() {
        let lines: Vec<String> = vec![];
        let m = analyze_function_lines(&lines, Language::Python);
        assert_eq!(m.loc, 0);
        assert_eq!(m.sloc, 0);
        assert_eq!(m.cyclomatic, 1); // base complexity
    }

    #[test]
    fn test_analyze_only_comments() {
        let lines: Vec<String> = vec![
            "# comment only".to_string(),
            "# another comment".to_string(),
        ];
        let m = analyze_function_lines(&lines, Language::Python);
        assert_eq!(m.loc, 2);
        assert_eq!(m.sloc, 0);
    }

    #[test]
    fn test_high_cyclomatic() {
        let lines: Vec<String> = vec![
            "def complex(x):".to_string(),
            "    if x > 0:".to_string(),
            "        if x > 10:".to_string(),
            "            for i in range(x):".to_string(),
            "                while True:".to_string(),
            "                    if i == 5:".to_string(),
            "                        break".to_string(),
            "    elif x < 0:".to_string(),
            "        return -1".to_string(),
            "    return 0".to_string(),
        ];
        let m = analyze_function_lines(&lines, Language::Python);
        assert!(m.cyclomatic >= 5);
    }
}
