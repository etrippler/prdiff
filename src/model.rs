use ratatui::prelude::Color;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub status: FileStatus,
    pub additions: i32,
    pub deletions: i32,
}

#[derive(Clone, Copy, Debug)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Unknown,
}

impl FileStatus {
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Added => "+",
            Self::Modified => "~",
            Self::Deleted => "-",
            Self::Renamed => "→",
            Self::Unknown => "?",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Added => Color::Green,
            Self::Modified => Color::Yellow,
            Self::Deleted => Color::Red,
            Self::Renamed => Color::Cyan,
            Self::Unknown => Color::Gray,
        }
    }
}

#[derive(Debug)]
pub enum TreeNode {
    Directory {
        name: String,
        children: Vec<TreeNode>,
    },
    File(FileEntry),
}

impl TreeNode {
    pub fn name(&self) -> &str {
        match self {
            Self::Directory { name, .. } => name,
            Self::File(f) => f.path.rsplit('/').next().unwrap_or(&f.path),
        }
    }
}

/// Pre-rendered diff line with syntax highlighting.
#[derive(Clone)]
pub struct HighlightedLine {
    pub spans: Vec<(String, Color, Color)>, // (text, fg, bg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSource {
    Worktree,
    Index,
    Untracked,
}
