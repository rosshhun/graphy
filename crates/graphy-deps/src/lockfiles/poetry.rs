//! poetry.lock parser (TOML format).

use anyhow::Result;

use super::Ecosystem;
use crate::DependencyInfo;

pub fn parse_poetry_lock(content: &str) -> Result<Vec<DependencyInfo>> {
    let parsed: toml::Value = content.parse()?;
    let mut deps = Vec::new();

    if let Some(packages) = parsed.get("package").and_then(|v| v.as_array()) {
        for pkg in packages {
            let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let category = pkg
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("main");

            if name.is_empty() || version.is_empty() {
                continue;
            }

            let is_optional = pkg
                .get("optional")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            deps.push(DependencyInfo {
                name: name.to_string(),
                version: version.to_string(),
                ecosystem: Ecosystem::Poetry,
                transitive: category != "main" || is_optional,
                parent: if category != "main" {
                    Some(category.to_string())
                } else {
                    None
                },
            });
        }
    }

    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_poetry_lock() {
        let content = r#"
[[package]]
name = "requests"
version = "2.31.0"
description = "Python HTTP for Humans."

[[package]]
name = "flask"
version = "3.0.0"
description = "A simple framework"
"#;

        let deps = parse_poetry_lock(content).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "requests");
        assert_eq!(deps[0].ecosystem, Ecosystem::Poetry);
    }

    #[test]
    fn parse_poetry_lock_empty() {
        let content = "";
        let deps = parse_poetry_lock(content).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_poetry_lock_dev_category() {
        let content = r#"
[[package]]
name = "pytest"
version = "7.4.0"
category = "dev"

[[package]]
name = "flask"
version = "3.0.0"
category = "main"
"#;
        let deps = parse_poetry_lock(content).unwrap();
        assert_eq!(deps.len(), 2);
        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.transitive); // dev deps marked as transitive
    }
}
