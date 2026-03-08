//! Dependency vulnerability mapping for Graphy.
//!
//! Parses lockfiles from various ecosystems, queries OSV.dev for known
//! vulnerabilities, and traces vulnerable dependency usage to call sites
//! in the code graph.

pub mod lockfiles;
pub mod osv;
pub mod trace;

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use graphy_core::CodeGraph;

pub use lockfiles::Ecosystem;
pub use osv::VulnEntry;
pub use trace::CallSite;

/// Information about a single dependency parsed from a lockfile.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyInfo {
    pub name: String,
    pub version: String,
    pub ecosystem: Ecosystem,
    pub transitive: bool,
    pub parent: Option<String>,
}

/// A full vulnerability report for a single dependency.
#[derive(Debug, Clone, Serialize)]
pub struct VulnerabilityReport {
    pub dependency: DependencyInfo,
    pub vulns: Vec<VulnEntry>,
    pub call_sites: Vec<CallSite>,
}

/// Detect lockfiles in a project root directory.
pub fn detect_lockfiles(root: &Path) -> Vec<(PathBuf, Ecosystem)> {
    let candidates = [
        ("Cargo.lock", Ecosystem::Cargo),
        ("package-lock.json", Ecosystem::Npm),
        ("yarn.lock", Ecosystem::Yarn),
        ("poetry.lock", Ecosystem::Poetry),
        ("go.sum", Ecosystem::Go),
    ];

    candidates
        .iter()
        .filter_map(|(name, eco)| {
            let path = root.join(name);
            if path.exists() {
                Some((path, *eco))
            } else {
                None
            }
        })
        .collect()
}

/// Parse a lockfile and return all dependencies.
pub fn parse_lockfile(path: &Path, eco: Ecosystem) -> Result<Vec<DependencyInfo>> {
    lockfiles::parse(path, eco)
}

/// Query OSV.dev for vulnerabilities in the given dependencies.
pub async fn query_vulns(deps: &[DependencyInfo]) -> Result<Vec<VulnerabilityReport>> {
    if deps.is_empty() {
        return Ok(Vec::new());
    }

    let vuln_map = osv::query_osv_batch(deps).await?;
    let mut reports = Vec::new();

    for dep in deps {
        let key = format!("{}@{}", dep.name, dep.version);
        if let Some(vulns) = vuln_map.get(&key) {
            if !vulns.is_empty() {
                reports.push(VulnerabilityReport {
                    dependency: dep.clone(),
                    vulns: vulns.clone(),
                    call_sites: Vec::new(),
                });
            }
        }
    }

    Ok(reports)
}

/// Full analysis: detect lockfiles, parse deps, check vulns, trace usage.
pub async fn analyze_dependencies(
    root: &Path,
    graph: Option<&CodeGraph>,
    check_vulns: bool,
) -> Result<DependencyAnalysis> {
    let lockfiles = detect_lockfiles(root);
    let mut all_deps = Vec::new();

    let mut parse_failures = 0usize;
    for (path, eco) in &lockfiles {
        match parse_lockfile(path, *eco) {
            Ok(deps) => all_deps.extend(deps),
            Err(e) => {
                tracing::warn!("Failed to parse {}: {}", path.display(), e);
                parse_failures += 1;
            }
        }
    }
    if !lockfiles.is_empty() && parse_failures == lockfiles.len() {
        tracing::error!(
            "All {} lockfile(s) failed to parse — dependency analysis will be empty",
            lockfiles.len()
        );
    }

    let mut reports = Vec::new();
    if check_vulns && !all_deps.is_empty() {
        match query_vulns(&all_deps).await {
            Ok(mut vulns) => {
                // Trace call sites if graph is available
                if let Some(g) = graph {
                    for report in &mut vulns {
                        report.call_sites = trace::trace_dep_usage(&report.dependency, g);
                    }
                }
                reports = vulns;
            }
            Err(e) => tracing::warn!("OSV query failed: {}", e),
        }
    }

    Ok(DependencyAnalysis {
        lockfiles_found: lockfiles.iter().map(|(p, e)| (p.clone(), *e)).collect(),
        total_deps: all_deps.len(),
        dependencies: all_deps,
        vulnerability_reports: reports,
    })
}

/// The result of a full dependency analysis.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyAnalysis {
    pub lockfiles_found: Vec<(PathBuf, Ecosystem)>,
    pub total_deps: usize,
    pub dependencies: Vec<DependencyInfo>,
    pub vulnerability_reports: Vec<VulnerabilityReport>,
}

/// Format the analysis as human-readable text.
pub fn format_deps_text(analysis: &DependencyAnalysis) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "Dependency Analysis: {} dependencies from {} lockfile(s)\n\n",
        analysis.total_deps,
        analysis.lockfiles_found.len()
    ));

    // Group by ecosystem
    let mut by_eco: std::collections::HashMap<Ecosystem, Vec<&DependencyInfo>> =
        std::collections::HashMap::new();
    for dep in &analysis.dependencies {
        by_eco.entry(dep.ecosystem).or_default().push(dep);
    }

    for (eco, deps) in &by_eco {
        out.push_str(&format!("{:?} ({} packages):\n", eco, deps.len()));
        for dep in deps.iter().take(20) {
            let marker = if dep.transitive { " (transitive)" } else { "" };
            out.push_str(&format!("  {} {}{}\n", dep.name, dep.version, marker));
        }
        if deps.len() > 20 {
            out.push_str(&format!("  ... and {} more\n", deps.len() - 20));
        }
        out.push('\n');
    }

    if !analysis.vulnerability_reports.is_empty() {
        out.push_str(&format!(
            "VULNERABILITIES ({} affected packages):\n\n",
            analysis.vulnerability_reports.len()
        ));
        for report in &analysis.vulnerability_reports {
            out.push_str(&format!(
                "  {} {} ({:?}):\n",
                report.dependency.name, report.dependency.version, report.dependency.ecosystem
            ));
            for vuln in &report.vulns {
                out.push_str(&format!(
                    "    [{}] {} (severity: {})\n",
                    vuln.id,
                    vuln.summary,
                    vuln.severity.as_deref().unwrap_or("unknown")
                ));
                if let Some(fixed) = &vuln.fixed_version {
                    out.push_str(&format!("      Fixed in: {}\n", fixed));
                }
            }
            if !report.call_sites.is_empty() {
                out.push_str(&format!(
                    "    Used at {} call sites:\n",
                    report.call_sites.len()
                ));
                for cs in report.call_sites.iter().take(5) {
                    let depth_label = match cs.depth {
                        0 => "import".to_string(),
                        1 => "direct".to_string(),
                        d => format!("{} hops", d),
                    };
                    out.push_str(&format!(
                        "      [{}] {} ({}:{})\n",
                        depth_label, cs.symbol_name, cs.file_path, cs.line
                    ));
                }
            }
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn detect_lockfiles_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let found = detect_lockfiles(tmp.path());
        assert!(found.is_empty());
    }

    #[test]
    fn detect_lockfiles_cargo() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.lock"), "# empty").unwrap();
        let found = detect_lockfiles(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, Ecosystem::Cargo);
    }

    #[test]
    fn detect_lockfiles_multiple() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.lock"), "").unwrap();
        fs::write(tmp.path().join("package-lock.json"), "{}").unwrap();
        fs::write(tmp.path().join("go.sum"), "").unwrap();
        let found = detect_lockfiles(tmp.path());
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn format_deps_text_empty() {
        let analysis = DependencyAnalysis {
            lockfiles_found: vec![],
            total_deps: 0,
            dependencies: vec![],
            vulnerability_reports: vec![],
        };
        let text = format_deps_text(&analysis);
        assert!(text.contains("0 dependencies"));
        assert!(text.contains("0 lockfile"));
    }

    #[test]
    fn format_deps_text_with_deps() {
        let analysis = DependencyAnalysis {
            lockfiles_found: vec![(PathBuf::from("Cargo.lock"), Ecosystem::Cargo)],
            total_deps: 2,
            dependencies: vec![
                DependencyInfo {
                    name: "serde".to_string(),
                    version: "1.0.0".to_string(),
                    ecosystem: Ecosystem::Cargo,
                    transitive: false,
                    parent: None,
                },
                DependencyInfo {
                    name: "serde_json".to_string(),
                    version: "1.0.0".to_string(),
                    ecosystem: Ecosystem::Cargo,
                    transitive: true,
                    parent: Some("serde".to_string()),
                },
            ],
            vulnerability_reports: vec![],
        };
        let text = format_deps_text(&analysis);
        assert!(text.contains("2 dependencies"));
        assert!(text.contains("serde"));
        assert!(text.contains("(transitive)"));
    }
}
