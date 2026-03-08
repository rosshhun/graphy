//! Phase 14: Git history analysis for change coupling.
//!
//! Shells out to `git log` to get commit history, parses which files changed
//! together, computes co-change frequency with temporal decay, and creates
//! COUPLED_WITH edges between frequently co-changed files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use graphy_core::{
    CodeGraph, EdgeKind, EdgeMetadata, GirEdge, NodeKind, SymbolId,
};
use tracing::{debug, warn};

/// Minimum co-change count to create a coupling edge.
const MIN_COCHANGE_COUNT: u32 = 2;

/// Temporal decay factor (lambda). Higher = faster decay.
const DECAY_LAMBDA: f64 = 0.005;

/// A co-change record between two files.
#[derive(Debug)]
struct CoChange {
    file_a: PathBuf,
    file_b: PathBuf,
    count: u32,
    temporal_weight: f64,
}

/// Phase 14: Analyze git history for change coupling.
pub fn analyze_change_coupling(graph: &mut CodeGraph, root: &Path, months: u32) {
    // Check if we're in a git repo
    let git_check = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output();

    match git_check {
        Ok(output) if output.status.success() => {}
        _ => {
            debug!("Phase 14 (Change Coupling): not a git repo, skipping");
            return;
        }
    }

    // Get commit history with changed files
    let since = format!("--since={} months ago", months);
    let log_output = Command::new("git")
        .args([
            "log",
            "--name-only",
            "--pretty=format:COMMIT:%H:%at",
            &since,
        ])
        .current_dir(root)
        .output();

    let output = match log_output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(o) => {
            warn!(
                "git log failed: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            return;
        }
        Err(e) => {
            warn!("Failed to run git log: {}", e);
            return;
        }
    };

    // Parse commits
    let commits = parse_git_log(&output);

    if commits.is_empty() {
        debug!("Phase 14 (Change Coupling): no commits found");
        return;
    }

    // Get the current time for temporal decay
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as f64;

    // Compute co-change frequencies with temporal decay
    let co_changes = compute_co_changes(&commits, now, root);

    // Build file path -> File node SymbolId map
    let mut file_node_map: HashMap<PathBuf, SymbolId> = HashMap::new();
    for node in graph.find_by_kind(NodeKind::File) {
        file_node_map.insert(node.file_path.clone(), node.id);
    }

    // Create COUPLED_WITH edges
    let mut edge_count = 0;
    for co_change in &co_changes {
        if co_change.count < MIN_COCHANGE_COUNT {
            continue;
        }

        let file_a_abs = root.join(&co_change.file_a);
        let file_b_abs = root.join(&co_change.file_b);

        let a_id = file_node_map
            .get(&file_a_abs)
            .or_else(|| file_node_map.get(&co_change.file_a));
        let b_id = file_node_map
            .get(&file_b_abs)
            .or_else(|| file_node_map.get(&co_change.file_b));

        if let (Some(&aid), Some(&bid)) = (a_id, b_id) {
            let edge = GirEdge::new(EdgeKind::CoupledWith)
                .with_confidence(coupling_confidence(co_change.count, co_change.temporal_weight))
                .with_metadata(EdgeMetadata::Coupling {
                    commit_count: co_change.count,
                    temporal_weight: co_change.temporal_weight,
                });
            graph.add_edge(aid, bid, edge);
            edge_count += 1;
        }
    }

    debug!(
        "Phase 14 (Change Coupling): {} commits analyzed, {} coupling edges created",
        commits.len(),
        edge_count
    );
}

/// A parsed commit from git log.
struct Commit {
    _hash: String,
    timestamp: f64,
    files: Vec<PathBuf>,
}

/// Parse the output of `git log --name-only --pretty=format:COMMIT:%H:%at`.
fn parse_git_log(output: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    let mut current_hash = String::new();
    let mut current_timestamp: f64 = 0.0;
    let mut current_files: Vec<PathBuf> = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("COMMIT:") {
            // Save previous commit if any
            if !current_hash.is_empty() && !current_files.is_empty() {
                commits.push(Commit {
                    _hash: current_hash.clone(),
                    timestamp: current_timestamp,
                    files: current_files.clone(),
                });
            }

            // Parse new commit header: COMMIT:hash:timestamp
            let parts: Vec<&str> = rest.splitn(2, ':').collect();
            current_hash = parts.first().unwrap_or(&"").to_string();
            current_timestamp = parts
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            current_files = Vec::new();
        } else {
            // This is a file path
            current_files.push(PathBuf::from(line));
        }
    }

    // Don't forget the last commit
    if !current_hash.is_empty() && !current_files.is_empty() {
        commits.push(Commit {
            _hash: current_hash,
            timestamp: current_timestamp,
            files: current_files,
        });
    }

    commits
}

/// Compute co-change frequencies with temporal decay.
fn compute_co_changes(commits: &[Commit], now: f64, _root: &Path) -> Vec<CoChange> {
    // For each pair of files that changed together, accumulate weighted counts.
    let mut pair_counts: HashMap<(PathBuf, PathBuf), (u32, f64)> = HashMap::new();

    for commit in commits {
        let days_ago = (now - commit.timestamp) / 86400.0;
        let weight = (-DECAY_LAMBDA * days_ago).exp();

        // Generate all pairs of files in this commit
        let files = &commit.files;
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                // Normalize pair order for consistent keys
                let (a, b) = if files[i] < files[j] {
                    (files[i].clone(), files[j].clone())
                } else {
                    (files[j].clone(), files[i].clone())
                };

                let entry = pair_counts.entry((a, b)).or_insert((0, 0.0));
                entry.0 += 1;
                entry.1 += weight;
            }
        }
    }

    pair_counts
        .into_iter()
        .map(|((file_a, file_b), (count, temporal_weight))| CoChange {
            file_a,
            file_b,
            count,
            temporal_weight,
        })
        .collect()
}

/// Compute coupling confidence based on co-change count and temporal weight.
fn coupling_confidence(count: u32, temporal_weight: f64) -> f32 {
    let count_factor = (count as f64 / 10.0).min(1.0);
    let time_factor = (temporal_weight / count as f64).min(1.0);
    ((count_factor * 0.6 + time_factor * 0.4) as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_log() {
        let log = "\
COMMIT:abc123:1700000000
src/main.py
src/utils.py

COMMIT:def456:1699000000
src/main.py
src/db.py
";
        let commits = parse_git_log(log);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].files.len(), 2);
        assert_eq!(commits[1].files.len(), 2);
    }

    #[test]
    fn test_coupling_confidence() {
        let high = coupling_confidence(10, 8.0);
        let low = coupling_confidence(2, 0.5);
        assert!(high > low);
    }

    #[test]
    fn test_parse_git_log_empty() {
        let commits = parse_git_log("");
        assert!(commits.is_empty());
    }

    #[test]
    fn test_parse_git_log_single_file() {
        let log = "COMMIT:abc:1700000000\nsrc/main.py\n";
        let commits = parse_git_log(log);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files.len(), 1);
    }

    #[test]
    fn test_coupling_confidence_single() {
        let c = coupling_confidence(1, 0.1);
        assert!(c > 0.0);
        assert!(c <= 1.0);
    }
}
