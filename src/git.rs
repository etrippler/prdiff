use crate::model::{DiffSource, FileEntry, FileStatus};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::process::Command;

pub fn detect_base_branch(specified: Option<String>) -> Result<String> {
    if let Some(b) = specified {
        return resolve_base_ref(&b);
    }

    // Try common base branch names. The upstream tracking branch isn't useful here
    // since feature branches typically track origin/feature-branch, not the base.
    for branch in ["develop", "main", "master"] {
        if let Ok(resolved) = resolve_base_ref(branch) {
            return Ok(resolved);
        }
    }
    anyhow::bail!(
        "No base branch found (develop/main/master). Specify one with --base <BRANCH>."
    )
}

pub fn get_merge_base(base: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["merge-base", "HEAD", base])
        .output()
        .context("Failed to run git merge-base")?;
    if !out.status.success() {
        anyhow::bail!("Could not find merge-base with '{base}'");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn get_changed_files(merge_base: &str) -> Result<Vec<FileEntry>> {
    // Effective PR diff is merge_base..(worktree) with a fallback to index-only changes
    // in the rare case the working tree no longer contains them.
    let work_status = git_diff_name_status(merge_base, false)?;
    let work_stats = git_diff_numstat(merge_base, false)?;
    let index_status = git_diff_name_status(merge_base, true)?;
    let index_stats = git_diff_numstat(merge_base, true)?;

    let mut files: Vec<FileEntry> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for (path, status) in &work_status {
        let (additions, deletions) = work_stats.get(path).copied().unwrap_or((0, 0));
        seen_paths.insert(path.clone());
        files.push(FileEntry {
            path: path.clone(),
            status: *status,
            additions,
            deletions,
        });
    }

    // Add index-only files that aren't represented in the working tree diff.
    for (path, status) in &index_status {
        if seen_paths.contains(path) {
            continue;
        }
        let (additions, deletions) = index_stats.get(path).copied().unwrap_or((0, 0));
        seen_paths.insert(path.clone());
        files.push(FileEntry {
            path: path.clone(),
            status: *status,
            additions,
            deletions,
        });
    }

    // Include untracked files
    let untracked_out = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()?;
    for line in String::from_utf8_lossy(&untracked_out.stdout).lines() {
        let path = line.trim().to_string();
        if path.is_empty() || seen_paths.contains(&path) {
            continue;
        }

        // Count lines for untracked files
        let line_count = std::fs::read(&path)
            .map(|bytes| {
                if bytes.is_empty() {
                    return 0;
                }
                let newlines = bytes.iter().filter(|b| **b == b'\n').count() as i32;
                let has_trailing_newline = bytes.last().copied() == Some(b'\n');
                if has_trailing_newline {
                    newlines
                } else {
                    newlines + 1
                }
            })
            .unwrap_or(0);

        files.push(FileEntry {
            path,
            status: FileStatus::Added,
            additions: line_count,
            deletions: 0,
        });
    }

    Ok(files)
}

pub fn get_file_diff(merge_base: &str, path: &str) -> (DiffSource, Vec<String>) {
    // Diff merge_base against working tree (not HEAD) to include uncommitted changes.
    // Fall back to index-only diff if the working tree doesn't currently contain the change.
    let worktree = Command::new("git")
        .args(["diff", merge_base, "--", path])
        .output();

    if let Ok(o) = worktree {
        let lines: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect();
        if !lines.is_empty() {
            return (DiffSource::Worktree, lines);
        }
    }

    let index = Command::new("git")
        .args(["diff", "--cached", merge_base, "--", path])
        .output();
    if let Ok(o) = index {
        let lines: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect();
        if !lines.is_empty() {
            return (DiffSource::Index, lines);
        }
    }

    // If git diff returns empty, file might be untracked - show as new file.
    if let Ok(bytes) = std::fs::read(path) {
        let mut result = vec![
            format!("diff --git a/{path} b/{path}"),
            "new file mode 100644".to_string(),
            "--- /dev/null".to_string(),
            format!("+++ b/{path}"),
        ];
        if bytes.is_empty() {
            result.push("@@ -0,0 +0,0 @@".to_string());
        } else {
            let content = String::from_utf8_lossy(&bytes);
            let file_lines: Vec<&str> = content.lines().collect();
            result.push(format!("@@ -0,0 +1,{} @@", file_lines.len()));
            for line in file_lines {
                result.push(format!("+{line}"));
            }
        }
        return (DiffSource::Untracked, result);
    }

    (DiffSource::Worktree, vec!["Error getting diff".to_string()])
}

pub fn git_git_path(name: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", name])
        .output()
        .with_context(|| format!("Failed to run git rev-parse --git-path {name}"))?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse --git-path {name} failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_rev_parse(rev: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", rev])
        .output()
        .with_context(|| format!("Failed to run git rev-parse {rev}"))?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse {rev} failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_status_hash() -> Result<u64> {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1", "-z"])
        .output()
        .context("Failed to run git status")?;
    if !out.status.success() {
        anyhow::bail!("git status failed");
    }
    Ok(hash_bytes(&out.stdout))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn file_mtime_ns(path: &str) -> Option<u128> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_nanos())
}

fn git_default_remote() -> Option<String> {
    let out = Command::new("git").args(["remote"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let remotes: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if remotes.iter().any(|r| r == "origin") {
        return Some("origin".to_string());
    }
    if remotes.len() == 1 {
        return Some(remotes[0].clone());
    }
    None
}

fn resolve_base_ref(specified: &str) -> Result<String> {
    if Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", specified])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(specified.to_string());
    }

    if !specified.contains('/') {
        if let Some(remote) = git_default_remote() {
            let candidate = format!("{remote}/{specified}");
            if Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", &candidate])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(candidate);
            }
        }
    }

    anyhow::bail!("Could not resolve base branch '{specified}'")
}

fn normalize_numstat_path(field: &str) -> String {
    // git --numstat for renames can emit either:
    // - "old\tnew" (extra tab-separated field)
    // - "dir/{old => new}/file" (brace expansion in a single field)
    // - "old => new" (single field)
    if let (Some(open), Some(close)) = (field.find('{'), field.rfind('}')) {
        if open < close {
            let prefix = &field[..open];
            let suffix = &field[close + 1..];
            let inner = &field[open + 1..close];
            if let Some((_, new)) = inner.split_once(" => ") {
                return format!("{prefix}{new}{suffix}");
            }
        }
    }
    if let Some((_, new)) = field.split_once(" => ") {
        return new.to_string();
    }
    field.to_string()
}

fn git_diff_name_status(merge_base: &str, cached: bool) -> Result<Vec<(String, FileStatus)>> {
    let mut args = vec!["diff", "--name-status"];
    if cached {
        args.push("--cached");
    }
    args.push(merge_base);

    let out = Command::new("git")
        .args(args)
        .output()
        .context("Failed to run git diff --name-status")?;
    if !out.status.success() {
        anyhow::bail!("git diff --name-status failed");
    }

    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let status_code = parts[0].chars().next().unwrap_or('?');
        let status = match status_code {
            'A' => FileStatus::Added,
            'M' | 'T' => FileStatus::Modified,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            'C' => FileStatus::Added,
            _ => FileStatus::Unknown,
        };
        let Some(path) = parts.last() else { continue };
        entries.push((path.to_string(), status));
    }
    Ok(entries)
}

fn git_diff_numstat(merge_base: &str, cached: bool) -> Result<HashMap<String, (i32, i32)>> {
    let mut args = vec!["diff", "--numstat"];
    if cached {
        args.push("--cached");
    }
    args.push(merge_base);

    let out = Command::new("git")
        .args(args)
        .output()
        .context("Failed to run git diff --numstat")?;
    if !out.status.success() {
        anyhow::bail!("git diff --numstat failed");
    }

    let mut stats: HashMap<String, (i32, i32)> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let add = parts[0].parse::<i32>().unwrap_or(0);
        let del = parts[1].parse::<i32>().unwrap_or(0);

        // If numstat includes both old and new paths (tab-separated), use the new path.
        let raw_path = if parts.len() >= 4 {
            parts[parts.len() - 1]
        } else {
            parts[2]
        };
        let path = normalize_numstat_path(raw_path);
        stats.insert(path, (add, del));
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::normalize_numstat_path;

    #[test]
    fn normalize_numstat_path_handles_brace_expansion() {
        assert_eq!(
            normalize_numstat_path("src/{old => new}/file.rs"),
            "src/new/file.rs"
        );
    }

    #[test]
    fn normalize_numstat_path_handles_simple_arrow() {
        assert_eq!(normalize_numstat_path("old => new"), "new");
    }
}
