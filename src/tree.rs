use crate::gitignore::{GitignoreRules, default_ignores};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TreeItem {
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
}

pub struct FileTree {
    pub root: PathBuf,
    pub items: Vec<TreeItem>,
    pub gitignore: Option<GitignoreRules>,
}

/// Directory nesting we scan below the root.
const MAX_DEPTH: usize = 20;

impl FileTree {
    pub fn build(root: &Path) -> Result<Self, String> {
        let gitignore = GitignoreRules::from_file(&root.join(".gitignore")).ok();
        let defaults = default_ignores();
        let mut items = Vec::new();

        Self::scan(root, root, 0, &mut items, &gitignore, &defaults);

        // dirs first (alphabetically), then files (alphabetically)
        items.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let an = a.path.file_name().unwrap().to_string_lossy();
                    let bn = b.path.file_name().unwrap().to_string_lossy();
                    an.cmp(&bn)
                }
            }
        });

        Ok(FileTree {
            root: root.to_path_buf(),
            items,
            gitignore,
        })
    }

    fn scan(
        root: &Path,
        current: &Path,
        depth: usize,
        items: &mut Vec<TreeItem>,
        gitignore: &Option<GitignoreRules>,
        defaults: &[&str],
    ) {
        let entries = match std::fs::read_dir(current) {
            Ok(e) => e,
            Err(e) => {
                // unreadable dir: skip it, keep the partial tree
                eprintln!("plat: skipping {}: {e}", current.display());
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            // file_type() does not follow symlinks, so a symlinked dir never
            // recurses (kills symlink loops for free)
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            if defaults.contains(&name.as_str()) {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path);
            if let Some(rules) = gitignore {
                if rules.matches(rel) {
                    continue;
                }
            }

            let is_dir = file_type.is_dir();
            items.push(TreeItem {
                path: path.clone(),
                is_dir,
                depth,
            });

            if is_dir && depth + 1 < MAX_DEPTH {
                Self::scan(root, &path, depth + 1, items, gitignore, defaults);
            }
        }
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        if let Some(rules) = &self.gitignore {
            if rules.matches(rel) {
                return true;
            }
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        default_ignores().contains(&name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Unique scratch dir under the temp dir (no tempfile crate: no new deps).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "plat-tree-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(tree: &FileTree) -> Vec<String> {
        tree.items
            .iter()
            .map(|i| {
                i.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn build_tree_on_simple_structure() {
        let dir = scratch("simple");
        fs::create_dir(dir.join("subdir")).unwrap();
        fs::write(dir.join("file1.txt"), "").unwrap();
        fs::write(dir.join("subdir").join("file2.txt"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        assert!(tree.items.len() >= 2);
        let got = names(&tree);
        assert!(got.contains(&"file1.txt".to_string()));
        assert!(got.contains(&"file2.txt".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_respects_gitignore() {
        let dir = scratch("ignore");
        fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        fs::write(dir.join("file.log"), "").unwrap();
        fs::write(dir.join("file.rs"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        let got = names(&tree);
        assert!(got.contains(&"file.rs".to_string()));
        assert!(!got.contains(&"file.log".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_sorts_dirs_first_then_files_alphabetically() {
        let dir = scratch("sort");
        fs::write(dir.join("z_file.txt"), "").unwrap();
        fs::create_dir(dir.join("a_dir")).unwrap();
        fs::write(dir.join("m_file.txt"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        let got = names(&tree);
        assert_eq!(got, vec!["a_dir", "m_file.txt", "z_file.txt"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_excludes_default_ignores() {
        let dir = scratch("defaults");
        fs::create_dir(dir.join(".git")).unwrap();
        fs::create_dir(dir.join("node_modules")).unwrap();
        fs::write(dir.join("file.rs"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        let got = names(&tree);
        assert!(!got.contains(&".git".to_string()));
        assert!(!got.contains(&"node_modules".to_string()));
        assert!(got.contains(&"file.rs".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_ignored_returns_true_for_ignored_paths() {
        let dir = scratch("isignored");
        fs::write(dir.join(".gitignore"), "*.log\n").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        assert!(tree.is_ignored(&dir.join("file.log")));
        assert!(!tree.is_ignored(&dir.join("file.rs")));

        fs::remove_dir_all(&dir).ok();
    }
}
