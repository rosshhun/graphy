//! package-lock.json parser (JSON format).

use anyhow::Result;
use serde_json::Value;

use super::Ecosystem;
use crate::DependencyInfo;

pub fn parse_package_lock(content: &str) -> Result<Vec<DependencyInfo>> {
    let parsed: Value = serde_json::from_str(content)?;
    let mut deps = Vec::new();

    // npm v2/v3 format uses "packages" field
    if let Some(packages) = parsed.get("packages").and_then(|v| v.as_object()) {
        for (key, pkg) in packages {
            // Skip the root "" entry
            if key.is_empty() {
                continue;
            }

            // Keys look like "node_modules/package-name" or "node_modules/@scope/package-name"
            let name = key
                .strip_prefix("node_modules/")
                .unwrap_or(key)
                .to_string();
            let version = pkg
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if name.is_empty() || version.is_empty() {
                continue;
            }

            let is_dev = pkg.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);

            deps.push(DependencyInfo {
                name,
                version,
                ecosystem: Ecosystem::Npm,
                // Nested node_modules indicate transitive deps in all npm lockfile versions.
                // "node_modules/foo" = direct, "node_modules/foo/node_modules/bar" = transitive.
                transitive: key
                    .strip_prefix("node_modules/")
                    .map_or(false, |rest| rest.contains("node_modules/")),
                parent: if is_dev {
                    Some("devDependencies".into())
                } else {
                    None
                },
            });
        }
    }
    // npm v1 format uses "dependencies" field
    else if let Some(dependencies) = parsed.get("dependencies").and_then(|v| v.as_object()) {
        parse_npm_v1_deps(dependencies, &mut deps, false);
    }

    Ok(deps)
}

fn parse_npm_v1_deps(
    deps_obj: &serde_json::Map<String, Value>,
    out: &mut Vec<DependencyInfo>,
    transitive: bool,
) {
    for (name, info) in deps_obj {
        let version = info
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !version.is_empty() {
            out.push(DependencyInfo {
                name: name.clone(),
                version,
                ecosystem: Ecosystem::Npm,
                transitive,
                parent: None,
            });
        }

        // Recurse into nested dependencies
        if let Some(nested) = info.get("dependencies").and_then(|v| v.as_object()) {
            parse_npm_v1_deps(nested, out, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_npm_v2_format() {
        let content = r#"{
            "name": "test-project",
            "lockfileVersion": 3,
            "packages": {
                "": { "name": "test-project", "version": "1.0.0" },
                "node_modules/express": { "version": "4.18.2" },
                "node_modules/lodash": { "version": "4.17.21", "dev": true }
            }
        }"#;

        let deps = parse_package_lock(content).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "express");
        assert_eq!(deps[0].version, "4.18.2");
    }

    #[test]
    fn parse_npm_empty_packages() {
        let content = r#"{"lockfileVersion": 3, "packages": {}}"#;
        let deps = parse_package_lock(content).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_npm_invalid_json() {
        let content = "not valid json";
        let result = parse_package_lock(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_npm_nested_transitive() {
        let content = r#"{
            "name": "test",
            "lockfileVersion": 3,
            "packages": {
                "": { "name": "test", "version": "1.0.0" },
                "node_modules/express": { "version": "4.18.2" },
                "node_modules/express/node_modules/body-parser": { "version": "1.20.0" }
            }
        }"#;
        let deps = parse_package_lock(content).unwrap();
        assert_eq!(deps.len(), 2);
        // Direct dep: express (1 level of node_modules)
        let express = deps.iter().find(|d| d.name == "express").unwrap();
        assert!(!express.transitive);
        // Transitive dep: nested under express/node_modules/
        let nested = deps.iter().find(|d| d.transitive).unwrap();
        assert!(nested.name.contains("body-parser"));
    }
}
