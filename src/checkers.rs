use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

use crate::types::Issue;

// --- Thresholds ---
pub const MAX_BODY_LINES: usize = 30;
pub const MAX_BODY_LINES_TEST: usize = 200;
pub const MAX_NESTING: usize = 4;
pub const MAX_FILE_LINES: usize = 750;

static FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\s*)(pub(?:\(crate\))?\s+)?(?:async\s+)?fn\s+(\w+)").unwrap()
});

static HEX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"0x[0-9a-fA-F]{3,}").unwrap());

// --- Test function detection ---

pub fn is_test_fn(lines: &[&str], fn_line: usize) -> bool {
    let mut idx = fn_line as isize - 1;
    while idx >= 0 {
        let t = lines[idx as usize].trim();
        if t.is_empty() {
            idx -= 1;
            continue;
        }
        if t == "#[test]" || t == "#[tokio::test]" {
            return true;
        }
        if !t.starts_with('#') {
            break;
        }
        idx -= 1;
    }
    false
}

// --- Checkers ---

pub fn check_file_length(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    if lines.len() > MAX_FILE_LINES {
        issues.push(Issue {
            category: "LENGTH".to_string(),
            file: rel(path, root),
            line: 1,
            function: None,
            problem: format!("File is {} lines (max {})", lines.len(), MAX_FILE_LINES),
            fix: "Split into submodules".to_string(),
        });
    }
    issues
}

pub fn check_functions(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let rel_path = rel(path, root);
    let mut i = 0;
    while i < lines.len() {
        let Some(caps) = FN_RE.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let fname = caps.get(3).unwrap().as_str().to_string();
        let fn_start = i;
        let is_test = is_test_fn(lines, i);

        // Find opening brace within 10 lines
        let search_end = (i + 10).min(lines.len());
        let found_open = (i..search_end).find(|&j| lines[j].contains('{'));
        let Some(j) = found_open else {
            i += 1;
            continue;
        };

        let (body_lines, max_depth, close_idx) = scan_fn_body(lines, j);
        let nesting = max_depth.saturating_sub(1); // fn body itself is depth 1
        let limit = if is_test { MAX_BODY_LINES_TEST } else { MAX_BODY_LINES };

        if body_lines > limit {
            issues.push(Issue {
                category: "LENGTH".to_string(),
                file: rel_path.clone(),
                line: fn_start + 1,
                function: Some(fname.clone()),
                problem: format!("{} body lines (max {})", body_lines, limit),
                fix: "Extract sequential steps into named helpers".to_string(),
            });
        }

        if nesting > MAX_NESTING && !is_test {
            issues.push(Issue {
                category: "NESTING".to_string(),
                file: rel_path.clone(),
                line: fn_start + 1,
                function: Some(fname.clone()),
                problem: format!("Nesting depth {} (max {})", nesting, MAX_NESTING),
                fix: "Use early returns, guard clauses, or extract inner blocks".to_string(),
            });
        }

        i = close_idx + 1;
    }
    issues
}

/// Returns (body_line_count, max_brace_depth, close_idx)
fn scan_fn_body(lines: &[&str], start: usize) -> (usize, usize, usize) {
    let mut brace_depth = 0usize;
    let mut max_depth = 0usize;
    let mut body_lines = 0usize;
    let mut opened = false;
    let mut close_idx = start;

    for k in start..lines.len() {
        for ch in lines[k].chars() {
            if ch == '{' {
                brace_depth += 1;
                opened = true;
                if brace_depth > max_depth {
                    max_depth = brace_depth;
                }
            } else if ch == '}' {
                brace_depth = brace_depth.saturating_sub(1);
            }
        }
        if opened && k > start && brace_depth > 0 {
            let stripped = lines[k].trim();
            if !stripped.is_empty()
                && !stripped.starts_with("//")
                && !stripped.starts_with('#')
                && !matches!(stripped, "{" | "}" | "};" | "}," | ");" | ")")
            {
                body_lines += 1;
            }
        }
        if opened && brace_depth == 0 {
            close_idx = k;
            break;
        }
    }
    (body_lines, max_depth, close_idx)
}

pub fn check_suppressions(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let rel_path = rel(path, root);
    for (i, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.contains("#[allow(clippy::too_many_arguments)]") {
            issues.push(Issue {
                category: "SUPPRESS".to_string(),
                file: rel_path.clone(),
                line: i + 1,
                function: None,
                problem: "Suppressed clippy::too_many_arguments".to_string(),
                fix: "Wrap related params into a context struct or SystemParam".to_string(),
            });
        } else if stripped.contains("#[allow(dead_code)]") {
            issues.push(Issue {
                category: "SUPPRESS".to_string(),
                file: rel_path.clone(),
                line: i + 1,
                function: None,
                problem: "Suppressed dead_code".to_string(),
                fix: "Use the code or delete it".to_string(),
            });
        } else if stripped.contains("#[allow(clippy::type_complexity)]") {
            issues.push(Issue {
                category: "SUPPRESS".to_string(),
                file: rel_path.clone(),
                line: i + 1,
                function: None,
                problem: "Suppressed clippy::type_complexity".to_string(),
                fix: "Create a type alias for the complex type".to_string(),
            });
        }
    }
    issues
}

pub fn check_state_accumulation(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let rel_path = rel(path, root);
    for (i, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.contains(".contains(") && i + 3 < lines.len() {
            for j in (i + 1)..(i + 4).min(lines.len()) {
                if lines[j].contains(".push(") {
                    issues.push(Issue {
                        category: "STATE".to_string(),
                        file: rel_path.clone(),
                        line: i + 1,
                        function: None,
                        problem: ".contains() + .push() is O(n^2)".to_string(),
                        fix: "Use HashSet or BTreeSet".to_string(),
                    });
                    break;
                }
            }
        }
    }
    issues
}

pub fn check_magic_numbers(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let rel_path = rel(path, root);
    for (i, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.starts_with("//")
            || stripped.starts_with("const ")
            || stripped.starts_with("pub const ")
        {
            continue;
        }
        for m in HEX_RE.find_iter(stripped) {
            if !stripped.contains("const ")
                && !stripped.contains("// ")
                && !stripped.contains("/// ")
            {
                issues.push(Issue {
                    category: "CLARITY".to_string(),
                    file: rel_path.clone(),
                    line: i + 1,
                    function: None,
                    problem: format!("Inline hex literal {}", m.as_str()),
                    fix: "Extract to named constant".to_string(),
                });
            }
        }
    }
    issues
}

fn rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

// --- Shared function extraction (used by similarity + duplicate) ---

pub const SKIP_NAMES: &[&str] = &[
    "main", "new", "default", "fmt", "from", "into", "drop", "clone", "build", "setup", "run",
];

/// Extract (name, start_line_1based, normalized_body_lines) for all functions.
pub fn extract_functions(lines: &[&str]) -> Vec<(String, usize, Vec<String>)> {
    let mut fns = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(caps) = FN_RE.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let fname = caps.get(3).unwrap().as_str().to_string();
        if SKIP_NAMES.contains(&fname.as_str()) || is_test_fn(lines, i) {
            i += 1;
            continue;
        }
        // Scan body
        let mut j = i;
        let mut brace_depth = 0usize;
        let mut opened = false;
        let mut body_lines: Vec<String> = Vec::new();
        while j < lines.len() {
            for ch in lines[j].chars() {
                if ch == '{' {
                    brace_depth += 1;
                    opened = true;
                } else if ch == '}' {
                    brace_depth = brace_depth.saturating_sub(1);
                }
            }
            if opened && j > i {
                body_lines.push(lines[j].trim().to_string());
            }
            if opened && brace_depth == 0 {
                break;
            }
            j += 1;
        }
        if body_lines.len() >= 5 {
            fns.push((fname, i + 1, body_lines));
        }
        i = j + 1;
    }
    fns
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/project")
    }

    fn fake_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/project/src/{}", name))
    }

    fn lines(src: &str) -> Vec<&str> {
        src.lines().collect()
    }

    // --- Long function detection ---

    #[test]
    fn test_long_function_flagged() {
        // Build a function with 31 non-trivial body lines
        let mut src = "fn big_fn() {\n".to_string();
        for i in 0..31 {
            src.push_str(&format!("    let x{} = {};\n", i, i));
        }
        src.push('}');
        let ls = lines(&src);
        let issues = check_functions(&fake_path("lib.rs"), &ls, &root());
        assert!(
            issues.iter().any(|i| i.category == "LENGTH"),
            "expected LENGTH issue"
        );
    }

    #[test]
    fn test_short_function_not_flagged() {
        let src = "fn small() {\n    let x = 1;\n    x\n}\n";
        let ls = lines(src);
        let issues = check_functions(&fake_path("lib.rs"), &ls, &root());
        assert!(issues.is_empty());
    }

    // --- Deep nesting detection ---

    #[test]
    fn test_deep_nesting_flagged() {
        let src = "fn deep() {\n\
            if a {\n\
                if b {\n\
                    if c {\n\
                        if d {\n\
                            if e {\n\
                                let x = 1;\n\
                            }\n\
                        }\n\
                    }\n\
                }\n\
            }\n\
        }\n";
        let ls = lines(src);
        let issues = check_functions(&fake_path("lib.rs"), &ls, &root());
        assert!(
            issues.iter().any(|i| i.category == "NESTING"),
            "expected NESTING issue"
        );
    }

    #[test]
    fn test_nesting_within_limit_not_flagged() {
        let src = "fn shallow() {\n\
            if a {\n\
                if b {\n\
                    let x = 1;\n\
                }\n\
            }\n\
        }\n";
        let ls = lines(src);
        let issues = check_functions(&fake_path("lib.rs"), &ls, &root());
        assert!(!issues.iter().any(|i| i.category == "NESTING"));
    }

    // --- Suppression detection ---

    #[test]
    fn test_suppress_too_many_args() {
        let src = "#[allow(clippy::too_many_arguments)]\nfn f() {}\n";
        let ls = lines(src);
        let issues = check_suppressions(&fake_path("lib.rs"), &ls, &root());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].problem.contains("too_many_arguments"));
    }

    #[test]
    fn test_suppress_dead_code() {
        let src = "#[allow(dead_code)]\nstruct Foo;\n";
        let ls = lines(src);
        let issues = check_suppressions(&fake_path("lib.rs"), &ls, &root());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].problem.contains("dead_code"));
    }

    #[test]
    fn test_suppress_type_complexity() {
        let src = "#[allow(clippy::type_complexity)]\ntype Foo = Vec<Vec<Vec<i32>>>;\n";
        let ls = lines(src);
        let issues = check_suppressions(&fake_path("lib.rs"), &ls, &root());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].problem.contains("type_complexity"));
    }

    // --- Hex literal detection ---

    #[test]
    fn test_hex_literal_flagged() {
        let src = "fn f() {\n    let x = 0xDEADBEEF;\n}\n";
        let ls = lines(src);
        let issues = check_magic_numbers(&fake_path("lib.rs"), &ls, &root());
        assert!(
            issues.iter().any(|i| i.problem.contains("0xDEADBEEF")),
            "expected hex literal issue"
        );
    }

    #[test]
    fn test_hex_in_const_not_flagged() {
        let src = "const MASK: u32 = 0xDEADBEEF;\n";
        let ls = lines(src);
        let issues = check_magic_numbers(&fake_path("lib.rs"), &ls, &root());
        assert!(issues.is_empty());
    }

    // --- .contains() + .push() pattern ---

    #[test]
    fn test_contains_push_flagged() {
        let src = "fn f(v: &mut Vec<i32>) {\n\
            if v.contains(&x) {\n\
                v.push(x);\n\
            }\n\
        }\n";
        let ls = lines(src);
        let issues = check_state_accumulation(&fake_path("lib.rs"), &ls, &root());
        assert!(!issues.is_empty(), "expected STATE issue");
    }

    #[test]
    fn test_contains_without_push_not_flagged() {
        let src = "fn f(v: &Vec<i32>) {\n    v.contains(&x);\n}\n";
        let ls = lines(src);
        let issues = check_state_accumulation(&fake_path("lib.rs"), &ls, &root());
        assert!(issues.is_empty());
    }

    // --- Dedup key generation ---

    #[test]
    fn test_dedup_key() {
        use crate::output::build_dedup_key;
        let issue = Issue {
            category: "LENGTH".to_string(),
            file: "src/foo.rs".to_string(),
            line: 42,
            function: Some("bar".to_string()),
            problem: "long".to_string(),
            fix: "fix it".to_string(),
        };
        assert_eq!(build_dedup_key(&issue), "LENGTH:src/foo.rs:bar");
    }

    #[test]
    fn test_dedup_key_no_fn() {
        use crate::output::build_dedup_key;
        let issue = Issue {
            category: "SUPPRESS".to_string(),
            file: "src/bar.rs".to_string(),
            line: 5,
            function: None,
            problem: "Suppressed dead_code".to_string(),
            fix: "fix".to_string(),
        };
        assert_eq!(build_dedup_key(&issue), "SUPPRESS:src/bar.rs:");
    }
}
