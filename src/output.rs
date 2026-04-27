use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

use crate::types::Issue;

pub const CATEGORY_ORDER: &[&str] = &[
    "COMPLEXITY",
    "LENGTH",
    "NESTING",
    "STATE",
    "SUPPRESS",
    "CLARITY",
    "SIBLING",
    "DUPLICATE",
];

pub fn format_text(issues: &[Issue]) -> String {
    let mut by_cat: HashMap<&str, Vec<&Issue>> = HashMap::new();
    for issue in issues {
        by_cat.entry(issue.category.as_str()).or_default().push(issue);
    }

    let mut lines = Vec::new();
    for cat in CATEGORY_ORDER {
        let Some(items) = by_cat.get(cat) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        lines.push(format!("\n## {} ({} issues)\n", cat, items.len()));
        let mut sorted: Vec<&&Issue> = items.iter().collect();
        sorted.sort_by_key(|i| (i.file.as_str(), i.line));
        for item in sorted {
            let fn_suffix = item
                .function
                .as_ref()
                .map(|f| format!(" — {}()", f))
                .unwrap_or_default();
            lines.push(format!(
                "  [{}] {}:{}{}",
                cat, item.file, item.line, fn_suffix
            ));
            lines.push(format!("    Problem: {}", item.problem));
            lines.push(format!("    Fix: {}", item.fix));
        }
    }
    lines.join("\n")
}

pub fn format_plan(issues: &[Issue]) -> String {
    let mut sorted: Vec<&Issue> = issues.iter().collect();
    sorted.sort_by_key(|i| (i.category.as_str(), i.file.as_str(), i.line));
    let lines: Vec<String> = sorted
        .iter()
        .map(|item| {
            let fn_prefix = item
                .function
                .as_ref()
                .map(|f| format!("`{}()` ", f))
                .unwrap_or_default();
            format!(
                "- [ ] `{}:{}` {}— {}",
                item.file, item.line, fn_prefix, item.problem
            )
        })
        .collect();
    lines.join("\n")
}

pub fn build_dedup_key(item: &Issue) -> String {
    let fn_part = item.function.as_deref().unwrap_or("");
    format!("{}:{}:{}", item.category, item.file, fn_part)
}

static PLAN_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^- \[.\] `([^`]+):(\d+)` (?:`(\w+)\(\)` )?— (.+)").unwrap()
});

fn guess_category(problem: &str) -> &'static str {
    if problem.contains("body lines") || problem.contains("lines (max") {
        return "LENGTH";
    }
    if problem.to_lowercase().contains("nesting") {
        return "NESTING";
    }
    if problem.to_lowercase().contains("cognitive") || problem.to_lowercase().contains("cyclomatic") {
        return "COMPLEXITY";
    }
    if problem.contains("contains()") || problem.contains("O(n") {
        return "STATE";
    }
    if problem.contains("Suppressed") || problem.contains("allow(") {
        return "SUPPRESS";
    }
    if problem.contains("hex literal") || problem.contains("Inline") {
        return "CLARITY";
    }
    if problem.to_lowercase().contains("similar") || problem.to_lowercase().contains("sibling") {
        return "SIBLING";
    }
    if problem.contains("Identical") || problem.to_lowercase().contains("duplicate") {
        return "DUPLICATE";
    }
    "LENGTH"
}

pub fn append_plan(root: &Path, issues: &[Issue]) -> anyhow::Result<usize> {
    let plan_path = root.join("PLAN.md");
    let existing = if plan_path.exists() {
        std::fs::read_to_string(&plan_path)?
    } else {
        String::new()
    };

    // Build set of existing dedup keys
    let mut existing_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for line in existing.lines() {
        if let Some(caps) = PLAN_LINE_RE.captures(line) {
            let fpath = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let fname = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            let problem = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            let cat = guess_category(problem);
            existing_keys.insert(format!("{}:{}:{}", cat, fpath, fname));
        }
    }

    let mut new_items: Vec<&Issue> = Vec::new();
    for item in issues {
        let key = build_dedup_key(item);
        if !existing_keys.contains(&key) {
            new_items.push(item);
            existing_keys.insert(key);
        }
    }

    if new_items.is_empty() {
        return Ok(0);
    }

    let mut by_cat: HashMap<&str, Vec<&&Issue>> = HashMap::new();
    for item in &new_items {
        by_cat.entry(item.category.as_str()).or_default().push(item);
    }

    let mut lines: Vec<String> = Vec::new();
    for cat in CATEGORY_ORDER {
        let Some(items) = by_cat.get(cat) else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        lines.push(format!("\n## Readability — {} (auto-detected)\n", title_case(cat)));
        let mut sorted: Vec<&&&Issue> = items.iter().collect();
        sorted.sort_by_key(|i| (i.file.as_str(), i.line));
        for item in sorted {
            let fn_prefix = item
                .function
                .as_ref()
                .map(|f| format!("`{}()` ", f))
                .unwrap_or_default();
            lines.push(format!(
                "- [ ] `{}:{}` {}— {}",
                item.file, item.line, fn_prefix, item.problem
            ));
        }
    }

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&plan_path)?;
    use std::io::Write;
    writeln!(f, "{}", lines.join("\n"))?;

    Ok(new_items.len())
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + &c.as_str().to_lowercase(),
    }
}
