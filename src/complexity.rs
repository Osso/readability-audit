use std::path::Path;

use serde_json::Value;

use crate::types::Issue;

const MAX_COGNITIVE: u64 = 15;
const MAX_CYCLOMATIC: u64 = 20;
const BINARY: &str = "rust-code-analysis-cli";

pub fn check_cognitive_complexity(root: &Path) -> Vec<Issue> {
    let src_dir = root.join("src");
    let result = std::process::Command::new(BINARY)
        .args(["-m", "-p", src_dir.to_str().unwrap_or("src"), "-O", "json"])
        .output();

    let output = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("warning: rust-code-analysis-cli failed to run: {}", e);
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
            eprintln!(
                "warning: rust-code-analysis-cli output was not valid UTF-8: {}",
                e
            );
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
        walk_complexity(
            &entry,
            entry["name"].as_str().unwrap_or(""),
            root,
            &mut issues,
        );
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
                problem: format!(
                    "Cyclomatic complexity {} (max {})",
                    cyclomatic, MAX_CYCLOMATIC
                ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn walks_nested_complexity_json_and_reports_thresholds() {
        let root = temp_dir("complexity-root");
        let file = root.join("src/lib.rs");
        let node = serde_json::json!({
            "kind": "module",
            "name": file,
            "spaces": [{
                "kind": "function",
                "name": "hard",
                "start_line": 12,
                "metrics": {
                    "cognitive": { "sum": 16.0 },
                    "cyclomatic": { "sum": 21.0 }
                }
            }, {
                "kind": "function",
                "name": "easy",
                "start_line": 2,
                "metrics": {
                    "cognitive": { "sum": 1.0 },
                    "cyclomatic": { "sum": 1.0 }
                }
            }]
        });

        let mut issues = Vec::new();
        walk_complexity(&node, file.to_str().unwrap(), &root, &mut issues);

        assert_eq!(issues.len(), 2);
        assert!(
            issues
                .iter()
                .any(|issue| issue.problem.contains("Cognitive"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.problem.contains("Cyclomatic"))
        );
        assert!(issues.iter().all(|issue| issue.file == "src/lib.rs"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn command_output_is_parsed_line_by_line() {
        let _lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let old_path = std::env::var("PATH").ok();
        let root = temp_dir("complexity-command");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let bin = temp_dir("complexity-bin");
        let script = bin.join(BINARY);
        let mut file = std::fs::File::create(&script).unwrap();
        writeln!(
            file,
            "#!/bin/sh\nprintf '%s\\n' '{{\"kind\":\"function\",\"name\":\"{}/src/main.rs\",\"start_line\":4,\"metrics\":{{\"cognitive\":{{\"sum\":17}},\"cyclomatic\":{{\"sum\":1}}}}}}'\nprintf '%s\\n' 'not-json'\nprintf '\\n'",
            root.display()
        )
        .unwrap();
        drop(file);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        unsafe {
            std::env::set_var("PATH", &bin);
        }

        let issues = check_cognitive_complexity(&root);
        assert_eq!(issues.len(), 1);
        let expected_function = format!("{}/src/main.rs", root.display());
        assert_eq!(
            issues[0].function.as_deref(),
            Some(expected_function.as_str())
        );

        unsafe {
            if let Some(path) = old_path {
                std::env::set_var("PATH", path);
            } else {
                std::env::remove_var("PATH");
            }
        }
        std::fs::remove_dir_all(root).ok();
        std::fs::remove_dir_all(bin).ok();
    }
}
