use std::collections::HashMap;
use std::path::Path;

use crate::checkers::extract_functions;
use crate::types::Issue;

const SIMILARITY_THRESHOLD: f64 = 0.75;
const MIN_BODY_LINES: usize = 8;

fn line_similarity(a: &[String], b: &[String]) -> f64 {
    use std::collections::HashSet;
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

pub fn check_sibling_similarity(path: &Path, lines: &[&str], root: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let rel_path = rel(path, root);

    // Skip test files
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.contains("_test") || stem.ends_with("_tests") {
        return issues;
    }

    let all_fns = extract_functions(lines);
    let fns: Vec<_> = all_fns
        .iter()
        .filter(|(_, _, body)| body.len() >= MIN_BODY_LINES)
        .collect();

    let mut seen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    for (i, (name_a, line_a, body_a)) in fns.iter().enumerate() {
        for (name_b, _line_b, body_b) in fns.iter().skip(i + 1) {
            let pair_key = (name_a.clone(), name_b.clone());
            if seen_pairs.contains(&pair_key) {
                continue;
            }
            let sim = line_similarity(body_a, body_b);
            if sim >= SIMILARITY_THRESHOLD {
                seen_pairs.insert(pair_key);
                let pct = (sim * 100.0) as usize;
                issues.push(Issue {
                    category: "SIBLING".to_string(),
                    file: rel_path.clone(),
                    line: *line_a,
                    function: Some(name_a.clone()),
                    problem: format!(
                        "`{}` and `{}` are {}% similar — likely copy-paste siblings",
                        name_a, name_b, pct
                    ),
                    fix: "Extract shared logic into a helper, keep only the differences in each function"
                        .to_string(),
                });
            }
        }
    }
    issues
}

pub fn check_duplicated_functions(root: &Path, files: &[std::path::PathBuf]) -> Vec<Issue> {
    // name -> Vec<(rel_path, body_string)>
    let mut fn_bodies: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for path in files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let line_vec: Vec<&str> = content.lines().collect();
        let rel_path = rel(path, root);
        for (fname, _, body_lines) in extract_functions(&line_vec) {
            fn_bodies
                .entry(fname)
                .or_default()
                .push((rel_path.clone(), body_lines.join("\n")));
        }
    }

    let mut issues = Vec::new();

    for (fname, entries) in &fn_bodies {
        if entries.len() < 2 {
            continue;
        }
        // Group by body content
        let mut body_groups: HashMap<&str, Vec<&str>> = HashMap::new();
        for (fpath, body) in entries {
            body_groups.entry(body.as_str()).or_default().push(fpath.as_str());
        }
        for (_, locs) in &body_groups {
            if locs.len() <= 1 {
                continue;
            }
            // Must span different parent directories
            let dirs: std::collections::HashSet<&str> = locs
                .iter()
                .map(|l| {
                    Path::new(l)
                        .parent()
                        .and_then(|p| p.to_str())
                        .unwrap_or("")
                })
                .collect();
            if dirs.len() > 1 {
                let locs_display = locs[..locs.len().min(3)].join(", ");
                issues.push(Issue {
                    category: "DUPLICATE".to_string(),
                    file: locs[0].to_string(),
                    line: 0,
                    function: Some(fname.clone()),
                    problem: format!(
                        "Identical function `{}` in {} files: {}",
                        fname,
                        locs.len(),
                        locs_display
                    ),
                    fix: "Extract into shared module".to_string(),
                });
            }
        }
    }
    issues
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

    fn make_fn(name: &str, lines: &[&str]) -> String {
        let body = lines.join("\n    ");
        format!("fn {}() {{\n    {}\n}}\n", name, body)
    }

    // --- Sibling similarity threshold ---

    #[test]
    fn test_sibling_similar_above_threshold() {
        // Two functions with highly similar bodies (differ only in one line)
        let body_lines: Vec<&str> = vec![
            "let a = foo();",
            "let b = bar();",
            "let c = baz();",
            "let d = qux();",
            "let e = quux();",
            "let f = corge();",
            "let g = grault();",
            "let h = garply();",
            "result_a",
        ];
        let body_lines_b: Vec<&str> = vec![
            "let a = foo();",
            "let b = bar();",
            "let c = baz();",
            "let d = qux();",
            "let e = quux();",
            "let f = corge();",
            "let g = grault();",
            "let h = garply();",
            "result_b",
        ];
        let src = format!(
            "{}\n{}",
            make_fn("func_a", &body_lines),
            make_fn("func_b", &body_lines_b)
        );
        let ls: Vec<&str> = src.lines().collect();
        let issues = check_sibling_similarity(&fake_path("lib.rs"), &ls, &root());
        assert!(
            !issues.is_empty(),
            "expected SIBLING issue for highly similar functions"
        );
    }

    #[test]
    fn test_sibling_dissimilar_below_threshold() {
        // Two functions with completely different bodies
        let src = "fn func_a() {\n\
            let a = alpha();\n\
            let b = beta();\n\
            let c = gamma();\n\
            let d = delta();\n\
            let e = epsilon();\n\
            let f = zeta();\n\
            let g = eta();\n\
            let h = theta();\n\
        }\n\
        fn func_b() {\n\
            let x = one();\n\
            let y = two();\n\
            let z = three();\n\
            let w = four();\n\
            let v = five();\n\
            let u = six();\n\
            let t = seven();\n\
            let s = eight();\n\
        }\n";
        let ls: Vec<&str> = src.lines().collect();
        let issues = check_sibling_similarity(&fake_path("lib.rs"), &ls, &root());
        assert!(
            issues.is_empty(),
            "expected no SIBLING issue for dissimilar functions"
        );
    }
}
