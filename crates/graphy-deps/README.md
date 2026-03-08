# graphy-deps

Dependency analysis and vulnerability scanning for [graphy](https://github.com/rosshhun/graphy).

## Overview

Parses lockfiles from 5 ecosystems, queries OSV.dev for known vulnerabilities, and traces vulnerable dependencies to their actual call sites in the code graph.

## Usage

```rust
use graphy_deps::analyze_dependencies;

let analysis = analyze_dependencies(&project_root, Some(&graph), true).await?;

println!("{}", graphy_deps::format_deps_text(&analysis));
```

## Pipeline

```
Project root
    |
    v
Detect lockfiles (Cargo.lock, package-lock.json, yarn.lock, poetry.lock, go.sum)
    |
    v
Parse dependencies (name, version, ecosystem)
    |
    v
Query OSV.dev for vulnerabilities (optional)
    |
    v
Trace to call sites in code graph (optional, requires indexed graph)
    |
    v
DependencyAnalysis result
```

## Supported lockfiles

| Ecosystem | Lockfile | Transitive deps |
|-----------|----------|----------------|
| Rust | `Cargo.lock` | Yes |
| npm | `package-lock.json` | Yes |
| Yarn | `yarn.lock` | Yes |
| Python | `poetry.lock` | Yes |
| Go | `go.sum` | Yes |

## Key types

```rust
pub struct DependencyAnalysis {
    pub lockfiles_found: Vec<(PathBuf, Ecosystem)>,
    pub total_deps: usize,
    pub dependencies: Vec<DependencyInfo>,
    pub vulnerability_reports: Vec<VulnerabilityReport>,
}

pub struct VulnerabilityReport {
    pub dependency: DependencyInfo,
    pub vulnerabilities: Vec<VulnEntry>,
    pub call_sites: Vec<CallSite>,  // Where this dep is actually used
}
```

## Call site tracing

When a code graph is available, graphy-deps traces vulnerable dependencies through import edges to find the exact functions that use them. This tells you whether a CVE actually affects your code or is in an unused transitive dependency.

## Dependencies

graphy-core, reqwest 0.12 (OSV.dev API), toml 0.8, serde_json 1, tokio 1
