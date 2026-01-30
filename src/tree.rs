use crate::model::{FileEntry, TreeNode};
use std::collections::HashSet;

pub fn build_tree(files: &[FileEntry]) -> Vec<TreeNode> {
    let mut root: Vec<TreeNode> = Vec::new();

    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        insert_into_tree(&mut root, &parts, file.clone());
    }

    sort_tree(&mut root);
    compact_tree(&mut root);
    root
}

fn insert_into_tree(nodes: &mut Vec<TreeNode>, parts: &[&str], file: FileEntry) {
    if parts.len() == 1 {
        nodes.push(TreeNode::File(file));
        return;
    }

    let dir_name = parts[0];
    let existing = nodes
        .iter_mut()
        .find(|n| matches!(n, TreeNode::Directory { name, .. } if name == dir_name));

    match existing {
        Some(TreeNode::Directory { children, .. }) => {
            insert_into_tree(children, &parts[1..], file);
        }
        _ => {
            let mut children = Vec::new();
            insert_into_tree(&mut children, &parts[1..], file);
            nodes.push(TreeNode::Directory {
                name: dir_name.to_string(),
                children,
            });
        }
    }
}

fn sort_tree(nodes: &mut Vec<TreeNode>) {
    nodes.sort_by(|a, b| {
        let a_is_dir = matches!(a, TreeNode::Directory { .. });
        let b_is_dir = matches!(b, TreeNode::Directory { .. });
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name().cmp(b.name()),
        }
    });
    for node in nodes {
        if let TreeNode::Directory { children, .. } = node {
            sort_tree(children);
        }
    }
}

/// Collapse single-child directory chains into one node
/// e.g., javascript/src/web/views/ becomes one directory node
/// Only merge if child directory also has exactly 1 child (pure chain)
pub fn compact_tree(nodes: &mut [TreeNode]) {
    for node in nodes.iter_mut() {
        if let TreeNode::Directory { name, children } = node {
            // Recursively compact children first
            compact_tree(children);

            // Only merge if: we have 1 child, it's a directory, AND it has exactly 1 child
            // This preserves branching points (directories with multiple children)
            loop {
                let should_merge = children.len() == 1
                    && matches!(
                        children.first(),
                        Some(TreeNode::Directory { children: gc, .. }) if gc.len() == 1
                    );

                if should_merge {
                    if let Some(TreeNode::Directory {
                        name: child_name,
                        children: grandchildren,
                    }) = children.pop()
                    {
                        *name = format!("{name}/{child_name}");
                        *children = grandchildren;
                    }
                } else {
                    break;
                }
            }
        }
    }
}

pub fn expand_all_dirs(nodes: &[TreeNode], prefix: &str, expanded: &mut HashSet<String>) {
    for node in nodes {
        if let TreeNode::Directory { name, children, .. } = node {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            expanded.insert(path.clone());
            expand_all_dirs(children, &path, expanded);
        }
    }
}

pub fn collect_visible<'a>(
    nodes: &'a [TreeNode],
    prefix: &str,
    depth: usize,
    expanded: &HashSet<String>,
    out: &mut Vec<(usize, String, &'a TreeNode)>,
) {
    for node in nodes {
        let path = match node {
            TreeNode::Directory { name, .. } => {
                if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                }
            }
            TreeNode::File(f) => f.path.clone(),
        };
        out.push((depth, path.clone(), node));

        if let TreeNode::Directory { children, .. } = node {
            if expanded.contains(&path) {
                collect_visible(children, &path, depth + 1, expanded, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tree, compact_tree};
    use crate::model::{FileEntry, FileStatus, TreeNode};

    #[test]
    fn compact_tree_does_not_merge_branching_directories() {
        let files = vec![
            FileEntry {
                path: "a/b/c/file1.txt".to_string(),
                status: FileStatus::Modified,
                additions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "a/b/d/file2.txt".to_string(),
                status: FileStatus::Modified,
                additions: 1,
                deletions: 0,
            },
        ];

        let tree = build_tree(&files);
        // "a/b" should exist as a directory because it branches into c and d.
        let root_dir = tree
            .iter()
            .find(|n| matches!(n, TreeNode::Directory { name, .. } if name == "a"));
        assert!(root_dir.is_some());
    }

    #[test]
    fn compact_tree_merges_pure_chains() {
        let mut nodes = vec![TreeNode::Directory {
            name: "a".to_string(),
            children: vec![TreeNode::Directory {
                name: "b".to_string(),
                children: vec![TreeNode::Directory {
                    name: "c".to_string(),
                    children: vec![TreeNode::File(FileEntry {
                        path: "a/b/c/file.txt".to_string(),
                        status: FileStatus::Modified,
                        additions: 0,
                        deletions: 0,
                    })],
                }],
            }],
        }];

        compact_tree(&mut nodes);
        let TreeNode::Directory { name, .. } = &nodes[0] else {
            panic!("expected directory");
        };
        assert_eq!(name, "a/b/c");
    }
}
