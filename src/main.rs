mod checkers;
mod complexity;
mod discovery;
mod output;
mod similarity;
mod types;

use std::collections::HashMap;
use clap::Parser;

use crate::types::Issue;

#[derive(Parser)]
#[command(name = "readability-audit", about = "Readability audit for a Rust repo")]
struct Args {
    /// Paths to scan (file or directory). Defaults to current directory.
    #[arg(value_name = "PATH")]
    paths: Vec<String>,

    /// Output PLAN.md-ready items (checkbox format)
    #[arg(long = "fix")]
    fix: bool,

    /// Output machine-readable JSON
    #[arg(long = "json")]
    json: bool,

    /// Append new items to repo's PLAN.md (deduped)
    #[arg(long = "write-plan")]
    write_plan: bool,

    /// Comma-separated dirs to skip (e.g. data,generated)
    #[arg(long = "exclude", value_name = "DIR1,DIR2")]
    exclude: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let extra_dirs: Vec<String> = args
        .exclude
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let (root, files, mut ignore) =
        discovery::resolve_scan_targets(&args.paths, &extra_dirs)?;

    if !root.join("Cargo.toml").exists() {
        eprintln!(
            "Error: {} is not a Rust project (no Cargo.toml)",
            root.display()
        );
        std::process::exit(1);
    }

    ignore.load_ignore_file(&root);

    eprintln!("Scanning {} Rust files in {}...", files.len(), root.display());

    let mut all_issues: Vec<Issue> = Vec::new();

    for path in &files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        all_issues.extend(checkers::check_file_length(path, &lines, &root));
        all_issues.extend(checkers::check_functions(path, &lines, &root));
        all_issues.extend(checkers::check_suppressions(path, &lines, &root));
        all_issues.extend(checkers::check_state_accumulation(path, &lines, &root));
        all_issues.extend(checkers::check_magic_numbers(path, &lines, &root));
        all_issues.extend(similarity::check_sibling_similarity(path, &lines, &root));
    }

    let complexity_issues = complexity::check_cognitive_complexity(&root);
    let selected_rel_paths: std::collections::HashSet<String> = files
        .iter()
        .filter_map(|p| {
            p.strip_prefix(&root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        })
        .collect();
    all_issues.extend(
        complexity_issues
            .into_iter()
            .filter(|i| selected_rel_paths.contains(&i.file)),
    );
    all_issues.extend(similarity::check_duplicated_functions(&root, &files));

    // Summary
    let mut by_cat: HashMap<&str, usize> = HashMap::new();
    for issue in &all_issues {
        *by_cat.entry(issue.category.as_str()).or_insert(0) += 1;
    }
    let total = all_issues.len();
    let mut summary_parts: Vec<String> = by_cat
        .iter()
        .map(|(cat, n)| format!("{}: {}", cat, n))
        .collect();
    summary_parts.sort();
    let summary = summary_parts.join(", ");
    eprintln!("\n{} issues found ({})", total, summary);

    if args.write_plan {
        let added = output::append_plan(&root, &all_issues)?;
        eprintln!("Added {} new items to {}", added, root.join("PLAN.md").display());
    } else if args.json {
        println!("{}", serde_json::to_string_pretty(&all_issues)?);
    } else if args.fix {
        println!("{}", output::format_plan(&all_issues));
    } else {
        println!("{}", output::format_text(&all_issues));
    }

    Ok(())
}
