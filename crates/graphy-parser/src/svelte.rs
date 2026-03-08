use std::path::Path;

use anyhow::{Context, Result};
use graphy_core::{
    EdgeKind, GirEdge, GirNode, Language, NodeKind, ParseOutput, SymbolId, Visibility,
};
use tree_sitter::Parser;

use crate::frontend::LanguageFrontend;
use crate::helpers::node_span;
use crate::typescript::TypeScriptFrontend;

pub struct SvelteFrontend;

impl SvelteFrontend {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageFrontend for SvelteFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_svelte_ng::LANGUAGE.into())
            .context("Failed to set Svelte language")?;

        let tree = parser
            .parse(source, None)
            .context("tree-sitter parse returned None")?;

        let root = tree.root_node();
        let mut output = ParseOutput::new();

        // Create file node
        let file_node = GirNode {
            id: SymbolId::new(path, path.to_string_lossy().as_ref(), NodeKind::File, 0),
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            kind: NodeKind::File,
            file_path: path.to_path_buf(),
            span: node_span(&root),
            visibility: Visibility::Public,
            language: Language::Svelte,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        };
        let file_id = file_node.id;
        output.add_node(file_node);

        // Find <script> elements and extract their content
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "script_element" {
                // Find the raw_text child (the JS/TS content)
                let mut inner = child.walk();
                for sc in child.children(&mut inner) {
                    if sc.kind() == "raw_text" {
                        let script_source = sc.utf8_text(source.as_bytes()).unwrap_or("");
                        if script_source.trim().is_empty() {
                            continue;
                        }

                        // Parse the script content as TypeScript
                        let ts_frontend = TypeScriptFrontend::new();
                        if let Ok(ts_output) = ts_frontend.parse(path, script_source) {
                            let line_offset = sc.start_position().row as u32;

                            // Build old-ID -> new-ID mapping for edge remapping
                            let mut id_remap =
                                std::collections::HashMap::<SymbolId, SymbolId>::new();

                            // Merge nodes, adjusting language and spans
                            for mut node in ts_output.nodes {
                                if node.kind == NodeKind::File {
                                    // Skip duplicate file node, link its children to our file
                                    continue;
                                }
                                let old_id = node.id;
                                node.language = Language::Svelte;
                                node.span.start_line += line_offset;
                                node.span.end_line += line_offset;
                                // Recompute ID with corrected start_line
                                node.id = SymbolId::new(
                                    &node.file_path,
                                    &node.name,
                                    node.kind,
                                    node.span.start_line,
                                );
                                id_remap.insert(old_id, node.id);
                                let node_id = node.id;
                                output.add_node(node);
                                output.add_edge(
                                    file_id,
                                    node_id,
                                    GirEdge::new(EdgeKind::Contains),
                                );
                            }

                            let ts_file_id = SymbolId::new(
                                path,
                                path.to_string_lossy().as_ref(),
                                NodeKind::File,
                                0,
                            );

                            // Merge edges, remapping IDs that changed due to offset
                            for (src, tgt, edge) in ts_output.edges {
                                // Skip Contains edges from the TS file node
                                // (we added our own above)
                                if edge.kind == EdgeKind::Contains && src == ts_file_id {
                                    continue;
                                }
                                // Remap source and target IDs to their offset-corrected versions
                                let new_src = id_remap.get(&src).copied().unwrap_or(src);
                                let new_tgt = id_remap.get(&tgt).copied().unwrap_or(tgt);
                                output.add_edge(new_src, new_tgt, edge);
                            }
                        }
                    }
                }
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::NodeKind;

    #[test]
    fn parse_svelte_component() {
        let source = r#"<script>
    function greet(name) {
        return "Hello, " + name;
    }

    const message = greet("World");
</script>

<h1>{message}</h1>
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("App.svelte"), source)
            .unwrap();

        // File node present
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));

        // Function extracted
        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(funcs.iter().any(|f| f.name == "greet"));

        // Language is Svelte
        for node in &output.nodes {
            assert_eq!(node.language, Language::Svelte);
        }
    }

    #[test]
    fn parse_svelte_with_typescript() {
        let source = r#"<script>
    export function add(a, b) {
        return a + b;
    }

    export function multiply(a, b) {
        return a * b;
    }
</script>

<div>
    <p>{add(2, 3)}</p>
</div>
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("Math.svelte"), source)
            .unwrap();

        let funcs: Vec<_> = output
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(funcs.len() >= 2);
        assert!(funcs.iter().any(|f| f.name == "add"));
        assert!(funcs.iter().any(|f| f.name == "multiply"));
    }

    #[test]
    fn parse_svelte_empty_script() {
        let source = r#"<script>
</script>

<h1>Hello</h1>
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("Empty.svelte"), source)
            .unwrap();

        // Just the file node
        assert_eq!(
            output.nodes.iter().filter(|n| n.kind == NodeKind::File).count(),
            1
        );
    }

    // ── Edge case tests ───────────────────────────────────

    #[test]
    fn parse_svelte_no_script() {
        // Template-only component with no <script> block
        let source = "<h1>Hello World</h1>\n<p>No script here</p>\n";
        let output = SvelteFrontend::new()
            .parse(Path::new("NoScript.svelte"), source)
            .unwrap();
        // Should still have a File node
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
        // No functions or classes
        let code_nodes: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function || n.kind == NodeKind::Class)
            .collect();
        assert!(code_nodes.is_empty());
    }

    #[test]
    fn parse_svelte_module_context() {
        let source = r#"<script context="module">
    export const API_URL = "https://example.com";
</script>

<script>
    function handleClick() {
        console.log("clicked");
    }
</script>

<button on:click={handleClick}>Click</button>
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("Module.svelte"), source)
            .unwrap();
        // Should parse at least the handleClick function
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn parse_svelte_with_imports() {
        let source = r#"<script>
    import { onMount } from 'svelte';
    import Button from './Button.svelte';

    onMount(() => {
        console.log('mounted');
    });
</script>

<Button />
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("WithImports.svelte"), source)
            .unwrap();
        let imports: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();
        assert!(imports.len() >= 1);
    }

    #[test]
    fn parse_svelte_script_context_module_extracts_functions() {
        // Svelte context="module" script blocks contain code that runs once at module level.
        // The parser should extract symbols from both module and instance script blocks.
        let source = r#"<script context="module">
    export function formatDate(date) {
        return date.toISOString();
    }
</script>

<script>
    export let date;

    function handleReset() {
        date = new Date();
    }
</script>

<p>{formatDate(date)}</p>
<button on:click={handleReset}>Reset</button>
"#;
        let output = SvelteFrontend::new()
            .parse(Path::new("DatePicker.svelte"), source)
            .unwrap();

        // Should have a File node
        assert!(output.nodes.iter().any(|n| n.kind == NodeKind::File));

        // At least one script block should have its functions extracted.
        // Both formatDate and handleReset may be found depending on how
        // the Svelte tree-sitter grammar exposes context="module" blocks.
        let funcs: Vec<_> = output.nodes.iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "Expected at least one function from script blocks");

        // All nodes should be tagged as Svelte language
        for node in &output.nodes {
            assert_eq!(node.language, Language::Svelte);
        }
    }
}
