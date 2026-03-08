//! go.sum parser (line-based format).

use anyhow::Result;
use std::collections::HashSet;

use super::Ecosystem;
use crate::DependencyInfo;

pub fn parse_go_sum(content: &str) -> Result<Vec<DependencyInfo>> {
    let mut seen = HashSet::new();
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Format: module version hash
        // e.g.: github.com/pkg/errors v0.9.1 h1:FE...=
        // or:   github.com/pkg/errors v0.9.1/go.mod h1:FE...=
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let module = parts[0];
        let version_str = parts[1];

        // Strip /go.mod suffix from version
        let version = version_str
            .strip_suffix("/go.mod")
            .unwrap_or(version_str);

        // Strip v prefix for clean version
        let clean_version = version.strip_prefix('v').unwrap_or(version);

        // Deduplicate (go.sum has two entries per module: one for source, one for go.mod)
        let key = format!("{}@{}", module, clean_version);
        if !seen.insert(key) {
            continue;
        }

        deps.push(DependencyInfo {
            name: module.to_string(),
            version: clean_version.to_string(),
            ecosystem: Ecosystem::Go,
            transitive: false,
            parent: None,
        });
    }

    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_go_sum() {
        let content = r#"github.com/pkg/errors v0.9.1 h1:FEBLx1zS214owpjy7qsBeixbURkuhQAwrK5UwLGTwt4=
github.com/pkg/errors v0.9.1/go.mod h1:bwawxfHBFNV+L2hUp1rHADufV3IMtnDRdf1r5NINEl0=
github.com/stretchr/testify v1.8.4 h1:CcVxjf3Q8PM0mHUKJCdn+eZZtm5yQksXRJi6DN2o+HA=
"#;

        let deps = parse_go_sum(content).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "github.com/pkg/errors");
        assert_eq!(deps[0].version, "0.9.1");
        assert_eq!(deps[0].ecosystem, Ecosystem::Go);
    }

    #[test]
    fn parse_go_sum_empty() {
        let deps = parse_go_sum("").unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_go_sum_short_lines_skipped() {
        // Lines with fewer than 3 whitespace-separated parts are skipped
        let content = "too_short\nanother\n\n";
        let deps = parse_go_sum(content).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_go_sum_deduplicates() {
        let content = r#"github.com/pkg/errors v0.9.1 h1:abc=
github.com/pkg/errors v0.9.1/go.mod h1:def=
"#;
        let deps = parse_go_sum(content).unwrap();
        // Same module+version should be deduplicated
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "github.com/pkg/errors");
    }
}
