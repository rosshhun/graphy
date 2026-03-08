//! OSV.dev batch API client for vulnerability queries.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::DependencyInfo;

const OSV_BATCH_URL: &str = "https://api.osv.dev/v1/querybatch";

/// Build a shared HTTP client with appropriate timeouts.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(2)
        .build()
        .expect("failed to build HTTP client")
}

/// A vulnerability entry from OSV.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnEntry {
    pub id: String,
    pub summary: String,
    pub severity: Option<String>,
    pub fixed_version: Option<String>,
}

/// OSV batch request format.
#[derive(Serialize)]
struct OsvBatchRequest {
    queries: Vec<OsvQuery>,
}

#[derive(Serialize)]
struct OsvQuery {
    package: OsvPackage,
    version: String,
}

#[derive(Serialize)]
struct OsvPackage {
    name: String,
    ecosystem: String,
}

/// OSV batch response format.
#[derive(Deserialize)]
struct OsvBatchResponse {
    results: Vec<OsvQueryResult>,
}

#[derive(Deserialize)]
struct OsvQueryResult {
    #[serde(default)]
    vulns: Vec<OsvVuln>,
}

#[derive(Deserialize)]
struct OsvVuln {
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
}

#[derive(Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    severity_type: String,
    score: String,
}

#[derive(Deserialize)]
struct OsvAffected {
    #[serde(default)]
    ranges: Vec<OsvRange>,
}

#[derive(Deserialize)]
struct OsvRange {
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Deserialize)]
struct OsvEvent {
    #[serde(default)]
    fixed: Option<String>,
}

/// Query OSV.dev in a single batch request.
/// Returns a map of "name@version" -> Vec<VulnEntry>.
pub async fn query_osv_batch(
    deps: &[DependencyInfo],
) -> Result<HashMap<String, Vec<VulnEntry>>> {
    let queries: Vec<OsvQuery> = deps
        .iter()
        .map(|dep| OsvQuery {
            package: OsvPackage {
                name: dep.name.clone(),
                ecosystem: dep.ecosystem.osv_ecosystem().to_string(),
            },
            version: dep.version.clone(),
        })
        .collect();

    let request = OsvBatchRequest { queries };

    let client = http_client();
    let response = client
        .post(OSV_BATCH_URL)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("OSV API returned status {}", response.status());
    }

    let batch_response: OsvBatchResponse = response.json().await?;

    let mut results: HashMap<String, Vec<VulnEntry>> = HashMap::new();

    for (i, query_result) in batch_response.results.iter().enumerate() {
        if i >= deps.len() {
            break;
        }
        let dep = &deps[i];
        let key = format!("{}@{}", dep.name, dep.version);

        let vulns: Vec<VulnEntry> = query_result
            .vulns
            .iter()
            .map(|v| {
                let severity = v
                    .severity
                    .first()
                    .map(|s| format!("{}: {}", s.severity_type, s.score));

                let fixed_version = v.affected.iter().find_map(|a| {
                    a.ranges.iter().find_map(|r| {
                        r.events.iter().find_map(|e| e.fixed.clone())
                    })
                });

                VulnEntry {
                    id: v.id.clone(),
                    summary: v.summary.clone(),
                    severity,
                    fixed_version,
                }
            })
            .collect();

        results.insert(key, vulns);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vuln_entry_serialization_roundtrip() {
        let entry = VulnEntry {
            id: "GHSA-1234".to_string(),
            summary: "Test vulnerability".to_string(),
            severity: Some("CVSS_V3: 7.5".to_string()),
            fixed_version: Some("1.2.3".to_string()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: VulnEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "GHSA-1234");
        assert_eq!(parsed.summary, "Test vulnerability");
        assert_eq!(parsed.severity.unwrap(), "CVSS_V3: 7.5");
        assert_eq!(parsed.fixed_version.unwrap(), "1.2.3");
    }

    #[test]
    fn vuln_entry_no_severity() {
        let entry = VulnEntry {
            id: "CVE-2024-001".to_string(),
            summary: "Something bad".to_string(),
            severity: None,
            fixed_version: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("CVE-2024-001"));
    }

    #[test]
    fn osv_batch_request_serialization() {
        let req = OsvBatchRequest {
            queries: vec![
                OsvQuery {
                    package: OsvPackage {
                        name: "serde".to_string(),
                        ecosystem: "crates.io".to_string(),
                    },
                    version: "1.0.0".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("serde"));
        assert!(json.contains("crates.io"));
        assert!(json.contains("1.0.0"));
    }

    #[test]
    fn osv_batch_response_deserialization_empty() {
        let json = r#"{"results": []}"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    #[test]
    fn osv_batch_response_deserialization_with_vulns() {
        let json = r#"{
            "results": [{
                "vulns": [{
                    "id": "GHSA-test",
                    "summary": "A test vuln",
                    "severity": [{"type": "CVSS_V3", "score": "7.0"}],
                    "affected": [{"ranges": [{"events": [{"fixed": "2.0.0"}]}]}]
                }]
            }]
        }"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].vulns.len(), 1);
        assert_eq!(resp.results[0].vulns[0].id, "GHSA-test");
        assert_eq!(resp.results[0].vulns[0].severity[0].score, "7.0");
    }

    #[test]
    fn osv_batch_response_deserialization_no_vulns() {
        let json = r#"{"results": [{"vulns": []}]}"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results[0].vulns.is_empty());
    }

    #[test]
    fn osv_query_result_default_vulns() {
        // Missing "vulns" field should default to empty
        let json = r#"{}"#;
        let result: OsvQueryResult = serde_json::from_str(json).unwrap();
        assert!(result.vulns.is_empty());
    }

    #[test]
    fn http_client_builds_successfully() {
        let client = http_client();
        // Just verify construction doesn't panic
        drop(client);
    }
}
