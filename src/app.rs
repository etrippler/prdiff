use crate::git;
use crate::highlight::Highlighter;
use crate::model::{DiffSource, FileEntry, HighlightedLine, TreeNode};
use crate::theme::Theme;
use crate::tree;
use crate::watcher::{GitWatcher, WatcherMessage};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::env;

pub struct App {
    pub files: Vec<FileEntry>,
    pub tree: Vec<TreeNode>,
    pub expanded: HashSet<String>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub diff_scroll: usize,
    pub diff_line_count: usize,
    diff_cache: HashMap<String, Vec<String>>,
    diff_source_cache: HashMap<String, DiffSource>,
    highlighted_cache: HashMap<String, Vec<HighlightedLine>>,
    pub merge_base: String,
    pub base_branch: String,
    editor: String,
    highlighter: Highlighter,
    tree_version: u64,
    watcher: GitWatcher,
    pub theme: Theme,
    /// Percentage of terminal width for the file tree panel (10-90)
    pub split_percent: u16,
}

impl App {
    pub fn new(base_branch: Option<String>, theme: Theme) -> Result<Self> {
        let base = git::detect_base_branch(base_branch)?;
        let merge_base = git::get_merge_base(&base)?;
        let files = git::get_changed_files(&merge_base)?;
        let tree = tree::build_tree(&files);
        let editor = env::var("PRDIFF_EDITOR")
            .or_else(|_| env::var("EDITOR"))
            .unwrap_or_else(|_| "zed".to_string());

        let mut expanded = HashSet::new();
        tree::expand_all_dirs(&tree, "", &mut expanded);

        // Spawn background watcher for git changes
        let watcher = GitWatcher::spawn(base.clone(), merge_base.clone(), files.clone());

        Ok(Self {
            files,
            tree,
            expanded,
            cursor: 0,
            scroll_offset: 0,
            diff_scroll: 0,
            diff_line_count: 0,
            diff_cache: HashMap::new(),
            diff_source_cache: HashMap::new(),
            highlighted_cache: HashMap::new(),
            merge_base,
            base_branch: base,
            editor,
            highlighter: Highlighter::new(theme),
            tree_version: 1,
            watcher,
            theme,
            split_percent: 30,
        })
    }

    /// Check for updates from the background watcher (non-blocking)
    pub fn check_for_changes(&mut self) {
        // Receive any updates from the background watcher (non-blocking)
        while let Some(msg) = self.watcher.try_recv() {
            match msg {
                WatcherMessage::FilesChanged {
                    files,
                    merge_base,
                    invalidate_all,
                    invalidate_paths,
                } => {
                    self.apply_file_changes(files, merge_base, invalidate_all, invalidate_paths);
                }
            }
        }
    }

    fn apply_file_changes(
        &mut self,
        files: Vec<FileEntry>,
        merge_base: String,
        invalidate_all: bool,
        invalidate_paths: HashSet<String>,
    ) {
        // Invalidate caches
        if invalidate_all {
            self.diff_cache.clear();
            self.diff_source_cache.clear();
            self.highlighted_cache.clear();
        } else {
            for path in &invalidate_paths {
                self.diff_cache.remove(path);
                self.diff_source_cache.remove(path);
                self.highlighted_cache.remove(path);
            }
        }

        let old_selected = self.selected_path();
        let mut old_dirs = HashSet::new();
        tree::expand_all_dirs(&self.tree, "", &mut old_dirs);

        self.merge_base = merge_base;
        self.files = files;
        self.tree = tree::build_tree(&self.files);
        self.tree_version = self.tree_version.wrapping_add(1);

        // Preserve user expand/collapse state for existing directories, but default-expand
        // any newly introduced directory nodes.
        let mut new_dirs = HashSet::new();
        tree::expand_all_dirs(&self.tree, "", &mut new_dirs);
        self.expanded = self
            .expanded
            .intersection(&new_dirs)
            .cloned()
            .collect::<HashSet<_>>();
        for dir in new_dirs.difference(&old_dirs) {
            self.expanded.insert(dir.clone());
        }

        // Remove caches for paths that no longer exist in the diff set.
        let new_paths: HashSet<String> = self.files.iter().map(|f| f.path.clone()).collect();
        self.diff_cache.retain(|p, _| new_paths.contains(p));
        self.diff_source_cache.retain(|p, _| new_paths.contains(p));
        self.highlighted_cache.retain(|p, _| new_paths.contains(p));

        // Preserve cursor on the previously selected path if possible.
        let visible_count = self.visible_items().len();
        if let Some(ref selected) = old_selected {
            let new_idx = self
                .visible_items()
                .iter()
                .enumerate()
                .find(|(_, (_, path, _))| path == selected)
                .map(|(idx, _)| idx);
            if let Some(idx) = new_idx {
                self.cursor = idx;
            }
        }

        // Clamp cursor
        if self.cursor >= visible_count && visible_count > 0 {
            self.cursor = visible_count - 1;
        }
        if visible_count == 0 {
            self.cursor = 0;
            self.scroll_offset = 0;
            self.diff_scroll = 0;
        }
    }

    pub fn tree_version(&self) -> u64 {
        self.tree_version
    }

    pub fn visible_items(&self) -> Vec<(usize, String, &TreeNode)> {
        let mut items = Vec::new();
        tree::collect_visible(&self.tree, "", 0, &self.expanded, &mut items);
        items
    }

    pub fn selected_path(&self) -> Option<String> {
        let visible = self.visible_items();
        visible.get(self.cursor).map(|(_, path, _)| path.clone())
    }

    pub fn toggle_expand(&mut self) {
        let dir_path = {
            let visible = self.visible_items();
            match visible.get(self.cursor) {
                Some((_, path, TreeNode::Directory { .. })) => Some(path.clone()),
                _ => None,
            }
        };
        if let Some(path) = dir_path {
            if self.expanded.contains(&path) {
                self.expanded.remove(&path);
            } else {
                self.expanded.insert(path);
            }
            self.tree_version = self.tree_version.wrapping_add(1);
        }
    }

    pub fn collapse_selected(&mut self) {
        let path = {
            let visible = self.visible_items();
            visible.get(self.cursor).map(|(_, path, _)| path.clone())
        };
        if let Some(path) = path {
            if self.expanded.remove(&path) {
                self.tree_version = self.tree_version.wrapping_add(1);
            }
        }
    }

    pub fn ensure_highlighted(&mut self, path: &str) {
        if self.highlighted_cache.contains_key(path) {
            return;
        }

        if !self.diff_cache.contains_key(path) {
            let (source, diff) = git::get_file_diff(&self.merge_base, path);
            self.diff_cache.insert(path.to_string(), diff);
            self.diff_source_cache.insert(path.to_string(), source);
        }

        let Some(diff_lines) = self.diff_cache.get(path) else {
            return;
        };

        let highlighted = self.highlighter.highlight_diff(diff_lines, path);
        self.highlighted_cache.insert(path.to_string(), highlighted);
    }

    pub fn get_highlighted(&self, path: &str) -> &[HighlightedLine] {
        self.highlighted_cache
            .get(path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn get_diff_source(&self, path: &str) -> Option<DiffSource> {
        self.diff_source_cache.get(path).copied()
    }

    /// Returns the (editor, path) to open, if a file is selected.
    /// The caller is responsible for terminal restore/re-enter around spawning.
    pub fn editor_command(&self) -> Option<(String, String)> {
        let visible = self.visible_items();
        match visible.get(self.cursor) {
            Some((_, _, TreeNode::File(f))) => Some((self.editor.clone(), f.path.clone())),
            _ => None,
        }
    }
}
