use std::collections::HashSet;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Directory-style exclusions (populated from EXCLUDE_DIRS default + .readability-ignore)
static GENERATED_PREFIX: &str = "generated_";

pub struct IgnoreConfig {
    pub exclude_dirs: HashSet<String>,
    pub exclude_patterns: Vec<String>,
}

impl IgnoreConfig {
    pub fn new(extra_dirs: &[String]) -> Self {
        let mut exclude_dirs = HashSet::new();
        exclude_dirs.insert("data".to_string());
        for d in extra_dirs {
            exclude_dirs.insert(d.clone());
        }
        IgnoreConfig {
            exclude_dirs,
            exclude_patterns: Vec::new(),
        }
    }

    pub fn load_ignore_file(&mut self, root: &Path) {
        let ignore_path = root.join(".readability-ignore");
        let Ok(content) = std::fs::read_to_string(&ignore_path) else {
            return;
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Treat as directory exclusion if: has '/', no '*'/'?', no extension (no dot in last component)
            let has_slash = line.contains('/');
            let has_glob = line.contains('*') || line.contains('?');
            // "no extension" heuristic: last path component has no '.'
            let last_component = line.trim_end_matches('/').split('/').last().unwrap_or(line);
            let has_extension = last_component.contains('.');
            if has_slash && !has_glob && !has_extension {
                self.exclude_dirs.insert(line.trim_end_matches('/').to_string());
            } else {
                self.exclude_patterns.push(line.to_string());
            }
        }
    }

    fn is_excluded_by_pattern(&self, rel: &str) -> bool {
        use glob::Pattern;
        let basename = Path::new(rel).file_name().and_then(|s| s.to_str()).unwrap_or("");
        for pat in &self.exclude_patterns {
            if let Ok(p) = Pattern::new(pat) {
                if p.matches(rel) || p.matches(basename) {
                    return true;
                }
            }
        }
        false
    }

    pub fn should_skip_file(&self, root: &Path, path: &Path) -> bool {
        let Ok(rel_path) = path.strip_prefix(root) else {
            return false;
        };
        let rel = rel_path.to_string_lossy();

        if rel.starts_with("target/") || rel.contains("/target/") {
            return true;
        }
        if rel.starts_with(".worktrees/") || rel.starts_with(".claude/worktrees/") {
            return true;
        }
        for d in &self.exclude_dirs {
            let prefix_slash = format!("{}/", d);
            let prefix_sep = format!("{}{}", d, std::path::MAIN_SEPARATOR);
            if rel.starts_with(prefix_slash.as_str()) || rel.starts_with(prefix_sep.as_str()) {
                return true;
            }
        }
        if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
            if fname.to_lowercase().starts_with(GENERATED_PREFIX) {
                return true;
            }
        }
        if self.is_excluded_by_pattern(&rel) {
            return true;
        }
        false
    }
}

pub fn find_project_root(path: &Path) -> Option<PathBuf> {
    let mut candidate = path.canonicalize().ok()?;
    if candidate.is_file() {
        candidate = candidate.parent()?.to_path_buf();
    }
    loop {
        if candidate.join("Cargo.toml").exists() {
            return Some(candidate);
        }
        let parent = candidate.parent()?;
        if parent == candidate {
            return None;
        }
        candidate = parent.to_path_buf();
    }
}

pub fn find_rs_files(root: &Path, ignore: &IgnoreConfig) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name().into_iter().flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if ignore.should_skip_file(root, path) {
            continue;
        }
        files.push(path.to_path_buf());
    }
    files
}

pub fn resolve_scan_targets(
    targets: &[String],
    extra_dirs: &[String],
) -> anyhow::Result<(PathBuf, Vec<PathBuf>, IgnoreConfig)> {
    if targets.is_empty() {
        let root = std::env::current_dir()?.canonicalize()?;
        let ignore = IgnoreConfig::new(extra_dirs);
        let files = find_rs_files(&root, &ignore);
        return Ok((root, files, ignore));
    }

    let resolved: Vec<PathBuf> = targets
        .iter()
        .map(|t| {
            let p = PathBuf::from(t);
            p.canonicalize().unwrap_or(p)
        })
        .collect();

    if resolved.len() == 1 && resolved[0].is_dir() {
        let root = resolved[0].clone();
        let ignore = IgnoreConfig::new(extra_dirs);
        let files = find_rs_files(&root, &ignore);
        return Ok((root, files, ignore));
    }

    // Multi-target: all must share a single Cargo root
    let roots: Result<Vec<Option<PathBuf>>, _> = resolved
        .iter()
        .map(|t| Ok(find_project_root(t)))
        .collect::<anyhow::Result<Vec<_>>>();
    let roots = roots?;

    for (t, r) in resolved.iter().zip(roots.iter()) {
        if r.is_none() {
            anyhow::bail!("Error: could not find Cargo.toml for {}", t.display());
        }
    }

    let unique_roots: HashSet<PathBuf> = roots.into_iter().flatten().collect();
    if unique_roots.len() != 1 {
        let root_list: Vec<String> = {
            let mut v: Vec<String> = unique_roots.iter().map(|r| r.display().to_string()).collect();
            v.sort();
            v
        };
        anyhow::bail!(
            "Error: targets span multiple Rust projects: {}",
            root_list.join(", ")
        );
    }

    let root = unique_roots.into_iter().next().unwrap();
    let ignore = IgnoreConfig::new(extra_dirs);

    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for target in &resolved {
        let candidates: Vec<PathBuf> = if target.is_dir() {
            let mut v: Vec<PathBuf> = Vec::new();
            for entry in WalkDir::new(target).sort_by_file_name().into_iter().flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("rs") {
                    v.push(entry.path().to_path_buf());
                }
            }
            v
        } else {
            vec![target.clone()]
        };

        for candidate in candidates {
            if candidate.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            if !candidate.exists() {
                continue;
            }
            // Must be under root
            if candidate.strip_prefix(&root).is_err() && candidate != root {
                continue;
            }
            if ignore.should_skip_file(&root, &candidate) {
                continue;
            }
            if seen.contains(&candidate) {
                continue;
            }
            seen.insert(candidate.clone());
            files.push(candidate);
        }
    }

    files.sort();
    Ok((root, files, ignore))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a unique temp directory under /tmp, return its path.
    /// Caller is responsible for cleanup (tests don't bother — OS cleans up).
    fn make_tmp_dir(suffix: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ra-test-{}-{}",
            suffix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn make_rs(dir: &Path, name: &str, content: &str) {
        if let Some(parent) = dir.join(name).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn test_generated_prefix_skipped() {
        let tmp = make_tmp_dir("gen");
        fs::write(tmp.join("Cargo.toml"), "[package]").unwrap();
        make_rs(&tmp, "src/generated_foo.rs", "fn x() {}");
        make_rs(&tmp, "src/normal.rs", "fn y() {}");
        let ignore = IgnoreConfig::new(&[]);
        let files = find_rs_files(&tmp, &ignore);
        assert_eq!(files.len(), 1);
        assert!(files[0].to_str().unwrap().ends_with("normal.rs"));
    }

    #[test]
    fn test_target_dir_skipped() {
        let tmp = make_tmp_dir("target");
        fs::write(tmp.join("Cargo.toml"), "[package]").unwrap();
        make_rs(&tmp, "target/debug/build_artifact.rs", "fn z() {}");
        make_rs(&tmp, "src/lib.rs", "fn y() {}");
        let ignore = IgnoreConfig::new(&[]);
        let files = find_rs_files(&tmp, &ignore);
        assert_eq!(files.len(), 1);
    }
}
