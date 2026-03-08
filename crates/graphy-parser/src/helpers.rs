use graphy_core::Span;
use tree_sitter::Node;

pub fn node_text(node: &Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

pub fn node_span(node: &Node) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span::new(
        start.row as u32,
        start.column as u32,
        end.row as u32,
        end.column as u32,
    )
}

/// Strip comment markers (`//`, `///`, `/*`, `#`, `*`) from doc captures.
pub fn clean_doc_comment(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("///") {
                trimmed[3..].trim()
            } else if trimmed.starts_with("//!") {
                trimmed[3..].trim()
            } else if trimmed.starts_with("//") {
                trimmed[2..].trim()
            } else if trimmed.starts_with("/**") {
                trimmed[3..].trim()
            } else if trimmed == "*/" {
                ""
            } else if trimmed.starts_with("* ") {
                trimmed[2..].trim()
            } else if trimmed == "*" {
                ""
            } else if trimmed.starts_with('#') {
                trimmed[1..].trim()
            } else {
                trimmed
            }
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Language-agnostic check for whether a call expression name looks like a
/// method call on a local variable/expression, which typically represents a
/// standard library method chain (e.g. `result.map()`, `vec.push()`,
/// `self.method()`, `obj.clone()`) rather than a user-defined function call.
///
/// Returns `true` if the call should be skipped (noise).
///
/// This replaces per-language hardcoded noise lists with a structural
/// heuristic that works across all languages:
///
/// - Bare function calls (`foo()`) → keep (could be user-defined)
/// - Qualified module/type calls (`Module.func()`, `os.path.join()`) → keep
/// - Method calls on local variables (`self.x()`, `result.map()`) → skip
///
/// The heuristic: if the receiver (part before the last `.`) starts with
/// a lowercase letter and is a short identifier (not a module path), it's
/// likely a variable, not a module/class name.
pub fn is_noise_method_call(name: &str) -> bool {
    // `::` paths (Rust) are always module/type qualified — never noise
    if name.contains("::") {
        return false;
    }

    // Find the `.` separator for method calls
    let (receiver, _method) = if let Some(pos) = name.rfind('.') {
        (&name[..pos], &name[pos + 1..])
    } else {
        // No separator — bare function call, not a method chain
        return false;
    };

    // If receiver itself contains `.`, it's a deep chain like `a.b.c()`
    let root = receiver
        .split('.')
        .next()
        .unwrap_or(receiver);

    // `self` / `this` / `super` are always local instance calls
    if matches!(root, "self" | "this" | "super" | "cls") {
        return true;
    }

    // Count the number of dot-separated segments.
    // `obj.method()` → 2 segments (likely variable.method)
    // `os.path.join()` → 3 segments (likely module path)
    let segment_count = name.split('.').count();

    // Multi-segment paths (3+) are almost always module references,
    // not method chains on variables. Keep them.
    if segment_count >= 3 {
        return false;
    }

    // For 2-segment calls (receiver.method or Module::method):
    // If the root starts with lowercase, it's likely a local variable
    // (e.g. `result.map()`, `graph.all_nodes()`, `edge.weight()`)
    // If uppercase, it's likely a type/module (e.g. `HashMap::new()`,
    // `Path.join()`, `React.createElement()`)
    let first_char = root.chars().next().unwrap_or('a');
    first_char.is_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_c_style_doc() {
        let input = "/**\n * Hello world\n * @param x the value\n */";
        let cleaned = clean_doc_comment(input);
        assert_eq!(cleaned, "Hello world\n@param x the value");
    }

    #[test]
    fn clean_rust_doc() {
        let input = "/// Hello world\n/// Second line";
        let cleaned = clean_doc_comment(input);
        assert_eq!(cleaned, "Hello world\nSecond line");
    }

    #[test]
    fn clean_hash_doc() {
        let input = "# Hello\n# World";
        let cleaned = clean_doc_comment(input);
        assert_eq!(cleaned, "Hello\nWorld");
    }

    #[test]
    fn noise_method_call_basics() {
        // Local variable method calls → noise
        assert!(is_noise_method_call("result.map"));
        assert!(is_noise_method_call("graph.all_nodes"));
        assert!(is_noise_method_call("edge.weight"));
        assert!(is_noise_method_call("vec.push"));
        assert!(is_noise_method_call("self.method"));
        assert!(is_noise_method_call("this.setState"));

        // Deep chains (3+ segments) are kept — could be module paths
        assert!(!is_noise_method_call("node.name.clone"));
        assert!(!is_noise_method_call("result.unwrap().method"));

        // Bare function calls → NOT noise
        assert!(!is_noise_method_call("my_function"));
        assert!(!is_noise_method_call("resolve_calls"));

        // Type/module qualified calls → NOT noise
        assert!(!is_noise_method_call("HashMap::new"));
        assert!(!is_noise_method_call("Database::create"));
        assert!(!is_noise_method_call("Path.join"));
        assert!(!is_noise_method_call("React.createElement"));
        assert!(!is_noise_method_call("JSON.parse"));
        assert!(!is_noise_method_call("Math.floor"));

        // Module paths → NOT noise
        assert!(!is_noise_method_call("os.path.join"));
        assert!(!is_noise_method_call("bincode::serialize"));
        assert!(!is_noise_method_call("Ok"));
    }

    #[test]
    fn clean_doc_comment_empty_and_unicode() {
        // Empty input should produce empty output
        assert_eq!(clean_doc_comment(""), "");

        // Whitespace-only lines should be filtered out
        assert_eq!(clean_doc_comment("   \n   \n"), "");

        // Unicode content should be preserved
        let input = "/// Calculates π (pi) value\n/// Returns: 3.14159…";
        let cleaned = clean_doc_comment(input);
        assert!(cleaned.contains("π"));
        assert!(cleaned.contains("3.14159…"));

        // Mixed comment styles with unicode
        let input2 = "# 日本語のドキュメント\n# 関数の説明";
        let cleaned2 = clean_doc_comment(input2);
        assert_eq!(cleaned2, "日本語のドキュメント\n関数の説明");

        // Emoji in doc comments
        let input3 = "/// 🚀 Launch the rocket";
        let cleaned3 = clean_doc_comment(input3);
        assert!(cleaned3.contains("🚀"));
    }

    #[test]
    fn noise_method_call_empty_and_edge_cases() {
        // Empty string — not a method call, no crash
        assert!(!is_noise_method_call(""));

        // Leading dot: root is "" which has no first char → default 'a' (lowercase) → noise
        assert!(is_noise_method_call(".method"));

        // Trailing dot: receiver is "obj" (lowercase root) → noise
        assert!(is_noise_method_call("obj."));

        // cls is treated like self/this
        assert!(is_noise_method_call("cls.create"));

        // super is treated as noise
        assert!(is_noise_method_call("super.init"));

        // Single character receiver starting lowercase
        assert!(is_noise_method_call("x.foo"));

        // Underscore-prefixed receiver: '_' is not ascii_lowercase, so treated
        // as a type/module-like call (not noise)
        assert!(!is_noise_method_call("_private.call"));
    }
}
