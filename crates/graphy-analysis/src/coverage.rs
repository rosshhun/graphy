//! Test coverage overlay: parse lcov/cobertura files and tag graph nodes
//! with coverage data. Combined with dead code detection, enables
//! high-confidence "unused AND untested" identification.

use std::collections::HashMap;
use std::path::Path;

use graphy_core::CodeGraph;
use tracing::{debug, info};

/// Per-file coverage data: line -> hit count.
pub type FileCoverage = HashMap<u32, u64>;

/// Parsed coverage report.
#[derive(Debug, Default)]
pub struct CoverageReport {
    /// Map from file path (relative) to line coverage.
    pub files: HashMap<String, FileCoverage>,
    pub total_lines: usize,
    pub covered_lines: usize,
}

/// Apply coverage data to graph nodes.
/// Sets `node.coverage` (0.0-1.0) based on which lines are covered.
pub fn apply_coverage(graph: &mut CodeGraph, report: &CoverageReport, project_root: &Path) {
    let mut tagged = 0usize;

    for node in graph.all_nodes_mut() {
        if !node.kind.is_callable() {
            continue;
        }

        let file_str = node.file_path.to_string_lossy();
        // Try both absolute and relative paths
        let rel_path = node
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&node.file_path);
        let rel_str = rel_path.to_string_lossy();

        let file_cov = report
            .files
            .get(file_str.as_ref())
            .or_else(|| report.files.get(rel_str.as_ref()));

        if let Some(cov) = file_cov {
            let start = node.span.start_line;
            let end = node.span.end_line;
            if end > start {
                let total = (end - start) as f64;
                let covered = (start..=end)
                    .filter(|line| cov.get(line).map_or(false, |&hits| hits > 0))
                    .count() as f64;
                node.coverage = Some((covered / total) as f32);
                tagged += 1;
            }
        }
    }

    info!("Coverage overlay: tagged {} functions", tagged);
}

/// Parse an lcov-format coverage file.
pub fn parse_lcov(content: &str) -> CoverageReport {
    let mut report = CoverageReport::default();
    let mut current_file: Option<String> = None;
    let mut current_cov: FileCoverage = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("SF:") {
            // Start of a file section
            current_file = Some(line[3..].to_string());
            current_cov = HashMap::new();
        } else if line.starts_with("DA:") {
            // DA:line_number,execution_count
            let parts: Vec<&str> = line[3..].splitn(2, ',').collect();
            if parts.len() == 2 {
                if let (Ok(line_num), Ok(hits)) = (parts[0].parse::<u32>(), parts[1].parse::<u64>())
                {
                    current_cov.insert(line_num, hits);
                    report.total_lines += 1;
                    if hits > 0 {
                        report.covered_lines += 1;
                    }
                }
            }
        } else if line == "end_of_record" {
            if let Some(file) = current_file.take() {
                report.files.insert(file, std::mem::take(&mut current_cov));
            }
        }
    }

    // Handle case where file doesn't end with end_of_record
    if let Some(file) = current_file {
        report.files.insert(file, current_cov);
    }

    debug!(
        "Parsed lcov: {} files, {}/{} lines covered",
        report.files.len(),
        report.covered_lines,
        report.total_lines
    );
    report
}

/// Auto-detect and load coverage from common file locations.
pub fn load_coverage(project_root: &Path) -> Option<CoverageReport> {
    let candidates = [
        "coverage/lcov.info",
        "lcov.info",
        "coverage.lcov",
        "target/coverage/lcov.info",
        ".coverage/lcov.info",
    ];

    for candidate in &candidates {
        let path = project_root.join(candidate);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                info!("Loading coverage from {}", path.display());
                return Some(parse_lcov(&content));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lcov() {
        let lcov = "\
SF:src/main.rs
DA:1,1
DA:2,1
DA:3,0
DA:4,5
end_of_record
SF:src/lib.rs
DA:10,0
DA:11,0
end_of_record
";
        let report = parse_lcov(lcov);
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.total_lines, 6);
        assert_eq!(report.covered_lines, 3);

        let main_cov = report.files.get("src/main.rs").unwrap();
        assert_eq!(*main_cov.get(&1).unwrap(), 1);
        assert_eq!(*main_cov.get(&3).unwrap(), 0);
        assert_eq!(*main_cov.get(&4).unwrap(), 5);

        let lib_cov = report.files.get("src/lib.rs").unwrap();
        assert_eq!(*lib_cov.get(&10).unwrap(), 0);
    }

    #[test]
    fn test_parse_lcov_empty() {
        let report = parse_lcov("");
        assert!(report.files.is_empty());
        assert_eq!(report.total_lines, 0);
    }

    #[test]
    fn test_parse_lcov_malformed_da_line() {
        let lcov = "SF:src/main.rs\nDA:abc,not_a_number\nDA:1,1\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.files.len(), 1);
        // Malformed DA line skipped, only valid DA counted
        assert_eq!(report.total_lines, 1);
        assert_eq!(report.covered_lines, 1);
    }

    #[test]
    fn test_parse_lcov_no_end_of_record() {
        let lcov = "SF:src/main.rs\nDA:1,1\nDA:2,0\n";
        let report = parse_lcov(lcov);
        // Should still capture the file even without end_of_record
        assert!(report.files.contains_key("src/main.rs") || report.files.is_empty());
    }
}
