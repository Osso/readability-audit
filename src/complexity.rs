use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::types::Issue;

const MAX_COGNITIVE: u64 = 15;
const MAX_CYCLOMATIC: u64 = 20;

fn rust_code_analysis_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home)
        .join("Repos/rust-code-analysis/target/release/rust-code-analysis-cli")
}

pub fn check_cognitive_complexity(root: &Path) -> Vec<Issue> {
    let binary = rust_code_analysis_path();
    if !binary.exists() {
        return Vec::new();
    }

    let src_dir = root.join("src");
    let result = std::process::Command::new(&binary)
        .args(["-m", "-p", src_dir.to_str().unwrap_or("src"), "-O", "json"])
        .output();

    let output = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "warning: rust-code-analysis-cli failed to run: {}",
                e
            );
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "warning: rust-code-analysis-cli exited with status {}",
            output.status
        );
        return Vec::new();
    }

    let stdout = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("warning: rust-code-analysis-cli output was not valid UTF-8: {}", e);
            return Vec::new();
        }
    };

    let mut issues = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        walk_complexity(&entry, entry["name"].as_str().unwrap_or(""), root, &mut issues);
    }
    issues
}

fn walk_complexity(node: &Value, fpath: &str, root: &Path, issues: &mut Vec<Issue>) {
    let kind = node["kind"].as_str().unwrap_or("");
    let name = node["name"].as_str().unwrap_or("");
    let start = node["start_line"].as_u64().unwrap_or(0) as usize;

    if kind == "function" {
        let cognitive = node["metrics"]["cognitive"]["sum"]
            .as_f64()
            .map(|v| v as u64)
            .unwrap_or(0);
        let cyclomatic = node["metrics"]["cyclomatic"]["sum"]
            .as_f64()
            .map(|v| v as u64)
            .unwrap_or(0);

        let rel_path = Path::new(fpath)
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| fpath.to_string());

        if cognitive > MAX_COGNITIVE {
            issues.push(Issue {
                category: "COMPLEXITY".to_string(),
                file: rel_path.clone(),
                line: start,
                function: Some(name.to_string()),
                problem: format!("Cognitive complexity {} (max {})", cognitive, MAX_COGNITIVE),
                fix: "Reduce branching, extract conditions into named booleans or helper functions"
                    .to_string(),
            });
        }
        if cyclomatic > MAX_CYCLOMATIC {
            issues.push(Issue {
                category: "COMPLEXITY".to_string(),
                file: rel_path.clone(),
                line: start,
                function: Some(name.to_string()),
                problem: format!("Cyclomatic complexity {} (max {})", cyclomatic, MAX_CYCLOMATIC),
                fix: "Reduce number of code paths, use data tables or dispatch".to_string(),
            });
        }
    }

    if let Some(spaces) = node["spaces"].as_array() {
        for child in spaces {
            walk_complexity(child, fpath, root, issues);
        }
    }
}
