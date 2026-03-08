# graphy-parser

Tree-sitter parsing and dynamic grammar loading for [graphy](https://github.com/rosshhun/graphy).

## Overview

Parses source code into GIR (Graphy Intermediate Representation) nodes and edges. Supports 5 built-in languages with custom frontends and 8 dynamic languages loaded at runtime.

## Architecture

```
Source code
    |
    v
Language dispatch (parse_file)
    |
    +-- Built-in: PythonFrontend / TypeScriptFrontend / RustFrontend / SvelteFrontend
    |      Deep understanding of imports, decorators, type annotations
    |
    +-- Dynamic: TagsFrontend + tags.scm query
           Generic extraction via tree-sitter queries
    |
    v
ParseOutput (Vec<GirNode> + Vec<GirEdge>)
```

## Public API

### Parsing

```rust
// Parse a single file (auto-detects language from extension)
let output = graphy_parser::parse_file(&path, &source)?;

// Parse many files in parallel (rayon)
let results = graphy_parser::parse_files(&files);
```

### `LanguageFrontend` trait

All language parsers implement this trait:

```rust
pub trait LanguageFrontend {
    fn parse(&self, path: &Path, source: &str) -> Result<ParseOutput>;
}
```

Built-in implementations: `PythonFrontend`, `TypeScriptFrontend`, `RustFrontend`, `SvelteFrontend`, `TagsFrontend`.

### Dynamic grammar loading

```rust
// Check if a grammar is installed
graphy_parser::dynamic_loader::is_installed("go");

// Get info about a known grammar
let info = graphy_parser::dynamic_loader::grammar_info_by_name("java");

// Install a grammar (clones repo, compiles with cc, installs .so/.dylib)
graphy_parser::grammar_compiler::install_grammar(info)?;
```

Grammars are stored at `~/.config/graphy/grammars/<name>/parser.{so,dylib}` and loaded via `libloading` (dlopen).

## Built-in languages

| Language | Frontend | Capabilities |
|----------|----------|-------------|
| Python | `PythonFrontend` | Imports (dotted paths), decorators, type annotations, `__all__` |
| TypeScript/JS | `TypeScriptFrontend` | ES imports, JSX, decorators, type annotations |
| Rust | `RustFrontend` | `use` paths, `impl` blocks, trait implementations, macros |
| Svelte | `SvelteFrontend` | Script block extraction, component references |

## Dynamic languages

Installed via `graphy lang add <name>`. Uses `TagsFrontend` with `.scm` query files.

| Language | Extensions | Grammar source |
|----------|------------|----------------|
| Go | `.go` | tree-sitter-go |
| Java | `.java` | tree-sitter-java |
| PHP | `.php` | tree-sitter-php |
| C | `.c`, `.h` | tree-sitter-c |
| C++ | `.cpp`, `.cc`, `.hpp` | tree-sitter-cpp |
| C# | `.cs` | tree-sitter-c-sharp |
| Ruby | `.rb` | tree-sitter-ruby |
| Kotlin | `.kt`, `.kts` | tree-sitter-kotlin |

## Custom tag queries

Override how symbols are extracted by placing a `tags.scm` at `~/.config/graphy/grammars/<lang>/tags.scm`. See `config/tags/` for examples.

Supported captures: `@definition.function`, `@definition.method`, `@definition.class`, `@definition.interface`, `@definition.module`, `@definition.constant`, `@definition.decorator`, `@reference.call`, `@name`, `@doc`.

## Dependencies

tree-sitter 0.24, libloading 0.8, rayon 1, graphy-core
