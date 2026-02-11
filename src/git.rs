use crate::model::{DiffSource, FileEntry, FileStatus};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::process::Command;

/// Create a git Command with GIT_OPTIONAL_LOCKS=0 to avoid creating index.lock.
/// prdiff is read-only and should never lock the index, which would conflict
/// with user git operations in the same repo.
fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

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
    let out = git_cmd()
        .args(["merge-base", "HEAD", base])
        .output()
        .context("Failed to run git merge-base")?;
    if !out.status.success() {
        anyhow::bail!("Could not find merge-base with '{base}'");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Check if file content appears to be binary by looking for NUL bytes in the first 8KB.
fn is_binary(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(8192);
    bytes[..check_len].contains(&0)
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn get_changed_files(merge_base: &str) -> Result<Vec<FileEntry>> {
    // Effective PR diff is merge_base..(worktree) with a fallback to index-only changes
    // in the rare case the working tree no longer contains them.
    let work_files = git_diff_status_and_stats(merge_base, false)?;
    let index_files = git_diff_status_and_stats(merge_base, true)?;

    let mut files: Vec<FileEntry> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for entry in &work_files {
        seen_paths.insert(entry.path.clone());
        files.push(entry.clone());
    }

    // Add index-only files that aren't represented in the working tree diff.
    for entry in &index_files {
        if seen_paths.contains(&entry.path) {
            continue;
        }
        seen_paths.insert(entry.path.clone());
        files.push(entry.clone());
    }

    // Include untracked files (use -z for NUL-delimited output)
    let untracked_out = git_cmd()
        .args(["ls-files", "-z", "--others", "--exclude-standard"])
        .output()?;
    for part in String::from_utf8_lossy(&untracked_out.stdout).split('\0') {
        let path = part.to_string();
        if path.is_empty() || seen_paths.contains(&path) {
            continue;
        }

        // Count lines for untracked files (skip binary)
        let line_count = std::fs::read(&path)
            .map(|bytes| {
                if bytes.is_empty() || is_binary(&bytes) {
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
    let worktree = git_cmd()
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

    let index = git_cmd()
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
        } else if is_binary(&bytes) {
            result.push(format!("Binary file {path} ({})", format_size(bytes.len())));
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
    let out = git_cmd()
        .args(["rev-parse", "--git-path", name])
        .output()
        .with_context(|| format!("Failed to run git rev-parse --git-path {name}"))?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse --git-path {name} failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_rev_parse(rev: &str) -> Result<String> {
    let out = git_cmd()
        .args(["rev-parse", rev])
        .output()
        .with_context(|| format!("Failed to run git rev-parse {rev}"))?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse {rev} failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_status_hash() -> Result<u64> {
    let out = git_cmd()
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
    let out = git_cmd().args(["remote"]).output().ok()?;
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

pub fn resolve_base_ref(specified: &str) -> Result<String> {
    // Prefer remote tracking ref (e.g. origin/develop) over local branch.
    // PR diffs compare against the remote, and local branches are often stale.
    if !specified.contains('/') {
        if let Some(remote) = git_default_remote() {
            let candidate = format!("{remote}/{specified}");
            if git_cmd()
                .args(["rev-parse", "--verify", "--quiet", &candidate])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Ok(candidate);
            }
        }
    }

    if git_cmd()
        .args(["rev-parse", "--verify", "--quiet", specified])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(specified.to_string());
    }

    anyhow::bail!("Could not resolve base branch '{specified}'")
}

pub fn list_branches() -> Result<Vec<String>> {
    let out = git_cmd()
        .args(["branch", "-a", "--format=%(refname:short)"])
        .output()
        .context("Failed to run git branch -a")?;
    if !out.status.success() {
        anyhow::bail!("git branch -a failed");
    }
    let mut seen = HashSet::new();
    let mut branches: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.contains("HEAD") && seen.insert(s.clone()))
        .collect();
    branches.sort();
    Ok(branches)
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

/// Run a single `git diff -z --raw --numstat` to get both status codes and line counts.
/// With -z, fields are NUL-delimited for safe handling of paths with special characters.
/// --raw gives `:oldmode newmode oldhash newhash status\0path[\0path]` records.
/// --numstat gives `add\tdel\tpath\0` records (tabs within, NUL between).
fn git_diff_status_and_stats(merge_base: &str, cached: bool) -> Result<Vec<FileEntry>> {
    let mut args = vec!["diff", "-z", "--raw", "--numstat"];
    if cached {
        args.push("--cached");
    }
    args.push(merge_base);

    let out = git_cmd()
        .args(args)
        .output()
        .context("Failed to run git diff -z --raw --numstat")?;
    if !out.status.success() {
        anyhow::bail!("git diff -z --raw --numstat failed");
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = text.split('\0').collect();

    let mut status_map: HashMap<String, FileStatus> = HashMap::new();
    let mut stats_map: HashMap<String, (i32, i32)> = HashMap::new();
    let mut paths_ordered: Vec<String> = Vec::new();

    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        if part.starts_with(':') {
            // --raw format with -z: `:oldmode newmode oldhash newhash status\0path[\0path]`
            // Status token is the last space-separated field (e.g. "M", "R100", "C085").
            // Extract the first character as the status letter.
            let status_token = part.split_whitespace().last().unwrap_or("?");
            let status_char = status_token.chars().next().unwrap_or('?');
            let status = match status_char {
                'A' => FileStatus::Added,
                'M' | 'T' => FileStatus::Modified,
                'D' => FileStatus::Deleted,
                'R' | 'C' => {
                    // Renames/copies have two paths: old\0new
                    // Skip old path, use new path
                    i += 1; // skip old path
                    if i < parts.len() {
                        i += 1; // move to new path
                    }
                    let path = parts.get(i).unwrap_or(&"").to_string();
                    if !path.is_empty() && !status_map.contains_key(&path) {
                        paths_ordered.push(path.clone());
                    }
                    let s = if status_char == 'R' { FileStatus::Renamed } else { FileStatus::Added };
                    status_map.insert(path, s);
                    i += 1;
                    continue;
                }
                _ => FileStatus::Unknown,
            };

            i += 1;
            let path = parts.get(i).unwrap_or(&"").to_string();
            if !path.is_empty() {
                if !status_map.contains_key(&path) {
                    paths_ordered.push(path.clone());
                }
                status_map.insert(path, status);
            }
        } else if !part.is_empty() && (part.as_bytes()[0].is_ascii_digit() || part.starts_with('-')) {
            // numstat format with -z: `add\tdel\tpath` (tabs within the NUL-delimited field)
            // For renames/copies with -z: `add\tdel\t\0old_path\0new_path` — the path field
            // after the second tab is empty, and old/new paths follow as separate NUL parts.
            // Binary files show as `-\t-\tpath`.
            let fields: Vec<&str> = part.split('\t').collect();
            if fields.len() >= 3 {
                let add = fields[0].parse::<i32>().unwrap_or(0);
                let del = fields[1].parse::<i32>().unwrap_or(0);
                let raw_path = fields[2];
                if raw_path.is_empty() {
                    // Rename/copy: consume old\0new from subsequent NUL-delimited parts
                    i += 1; // skip old path
                    i += 1; // move to new path
                    let path = parts.get(i).unwrap_or(&"").to_string();
                    if !path.is_empty() {
                        stats_map.insert(path, (add, del));
                    }
                } else {
                    let path = normalize_numstat_path(raw_path);
                    stats_map.insert(path, (add, del));
                }
            }
        }
        i += 1;
    }

    let mut entries = Vec::new();
    for path in &paths_ordered {
        let status = status_map.get(path).copied().unwrap_or(FileStatus::Unknown);
        let (additions, deletions) = stats_map.get(path).copied().unwrap_or((0, 0));
        entries.push(FileEntry {
            path: path.clone(),
            status,
            additions,
            deletions,
        });
    }
    Ok(entries)
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
