use crate::git;
use crate::model::FileEntry;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Message sent from background watcher to main thread
pub enum WatcherMessage {
    /// Files have changed - here's the new state
    FilesChanged {
        files: Vec<FileEntry>,
        merge_base: String,
        invalidate_all: bool,
        invalidate_paths: HashSet<String>,
    },
}

/// Handle to the background watcher thread
pub struct GitWatcher {
    receiver: Receiver<WatcherMessage>,
    _handle: JoinHandle<()>,
}

impl GitWatcher {
    /// Spawn a background thread that watches for git changes
    pub fn spawn(base_branch: String, initial_merge_base: String, initial_files: Vec<FileEntry>) -> Self {
        let (sender, receiver) = mpsc::channel();

        let handle = thread::spawn(move || {
            watcher_loop(sender, base_branch, initial_merge_base, initial_files);
        });

        Self {
            receiver,
            _handle: handle,
        }
    }

    /// Check for updates from the background thread (non-blocking)
    pub fn try_recv(&self) -> Option<WatcherMessage> {
        self.receiver.try_recv().ok()
    }
}

fn watcher_loop(
    sender: Sender<WatcherMessage>,
    base_branch: String,
    mut merge_base: String,
    mut files: Vec<FileEntry>,
) {
    let mut last_head_oid = git::git_rev_parse("HEAD").unwrap_or_default();
    let mut last_base_oid = git::git_rev_parse(&base_branch).unwrap_or_default();
    let mut last_status_hash = git::git_status_hash().unwrap_or(0);
    let git_index_path = git::git_git_path("index").unwrap_or_default();
    let git_head_path = git::git_git_path("HEAD").unwrap_or_default();
    // Resolve the base branch ref path for cheap mtime checks.
    // For remote refs like "origin/main", this resolves to e.g. ".git/refs/remotes/origin/main"
    // or packed-refs. We also watch the packed-refs file for repacks.
    let git_refs_heads_path = git::git_git_path(&format!("refs/heads/{base_branch}")).unwrap_or_default();
    let git_refs_remotes_path = git::git_git_path(&format!("refs/remotes/{base_branch}")).unwrap_or_default();
    let git_packed_refs_path = git::git_git_path("packed-refs").unwrap_or_default();

    let mut last_index_mtime = git::file_mtime_ns(&git_index_path);
    let mut last_head_mtime = git::file_mtime_ns(&git_head_path);
    let mut last_refs_heads_mtime = git::file_mtime_ns(&git_refs_heads_path);
    let mut last_refs_remotes_mtime = git::file_mtime_ns(&git_refs_remotes_path);
    let mut last_packed_refs_mtime = git::file_mtime_ns(&git_packed_refs_path);
    let mut file_mtimes = get_file_mtimes(&files);

    loop {
        thread::sleep(Duration::from_millis(200));

        let mut invalidate_all_caches = false;
        let mut invalidate_paths: HashSet<String> = HashSet::new();
        let mut needs_refresh = false;

        // Cheap mtime checks on git internal files to avoid spawning processes
        let index_mtime = git::file_mtime_ns(&git_index_path);
        let head_mtime = git::file_mtime_ns(&git_head_path);
        let refs_heads_mtime = git::file_mtime_ns(&git_refs_heads_path);
        let refs_remotes_mtime = git::file_mtime_ns(&git_refs_remotes_path);
        let packed_refs_mtime = git::file_mtime_ns(&git_packed_refs_path);

        let git_dir_changed = index_mtime != last_index_mtime
            || head_mtime != last_head_mtime
            || refs_heads_mtime != last_refs_heads_mtime
            || refs_remotes_mtime != last_refs_remotes_mtime
            || packed_refs_mtime != last_packed_refs_mtime;

        if git_dir_changed {
            // Something in .git changed - check what exactly
            if index_mtime != last_index_mtime {
                last_index_mtime = index_mtime;
                invalidate_all_caches = true;
                needs_refresh = true;
            }

            if head_mtime != last_head_mtime
                || refs_heads_mtime != last_refs_heads_mtime
                || refs_remotes_mtime != last_refs_remotes_mtime
                || packed_refs_mtime != last_packed_refs_mtime
            {
                last_head_mtime = head_mtime;
                last_refs_heads_mtime = refs_heads_mtime;
                last_refs_remotes_mtime = refs_remotes_mtime;
                last_packed_refs_mtime = packed_refs_mtime;

                // Only spawn git processes when ref files actually changed
                let head_oid = match git::git_rev_parse("HEAD") {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let base_oid = match git::git_rev_parse(&base_branch) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if head_oid != last_head_oid || base_oid != last_base_oid {
                    invalidate_all_caches = true;
                    if let Ok(new_merge_base) = git::get_merge_base(&base_branch) {
                        merge_base = new_merge_base;
                        last_head_oid = head_oid;
                        last_base_oid = base_oid;
                        needs_refresh = true;
                    }
                }
            }
        }

        // Check file mtimes (cheap stat calls, no git processes)
        let new_mtimes = get_file_mtimes(&files);
        for path in files.iter().map(|f| f.path.as_str()) {
            let old = file_mtimes.get(path).copied();
            let new = new_mtimes.get(path).copied();
            if old != new {
                invalidate_paths.insert(path.to_string());
                needs_refresh = true;
            }
        }

        // Only run git status when file mtimes changed (detects new untracked files, staging)
        if !invalidate_paths.is_empty() || git_dir_changed {
            if let Ok(status_hash) = git::git_status_hash() {
                if status_hash != last_status_hash {
                    last_status_hash = status_hash;
                    needs_refresh = true;
                }
            }
        }

        if !needs_refresh {
            continue;
        }

        // Fetch new file list
        let new_files = match git::get_changed_files(&merge_base) {
            Ok(f) => f,
            Err(_) => continue,
        };

        file_mtimes = get_file_mtimes(&new_files);

        // Send update to main thread
        let msg = WatcherMessage::FilesChanged {
            files: new_files.clone(),
            merge_base: merge_base.clone(),
            invalidate_all: invalidate_all_caches,
            invalidate_paths,
        };

        files = new_files;

        if sender.send(msg).is_err() {
            // Main thread has dropped the receiver, exit
            break;
        }
    }
}

fn get_file_mtimes(files: &[FileEntry]) -> HashMap<String, u128> {
    let mut mtimes = HashMap::new();
    for file in files {
        if let Some(mtime) = git::file_mtime_ns(&file.path) {
            mtimes.insert(file.path.clone(), mtime);
        }
    }
    mtimes
}
