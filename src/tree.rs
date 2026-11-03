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
    /// directories the scan could not read (status-bar note)
    pub skipped: usize,
}

/// Directory nesting we scan below the root.
const MAX_DEPTH: usize = 20;

// ponytail: full synchronous scan at sidebar-open time (no lazy load, no
// thread, no `ignore` crate); revisit when a repo takes >100ms to scan.
impl FileTree {
    pub fn build(root: &Path) -> Result<Self, String> {
        let gitignore = GitignoreRules::from_file(&root.join(".gitignore")).ok();
        let defaults = default_ignores();
        let mut items = Vec::new();

        let skipped = Self::scan(root, root, 0, &mut items, &gitignore, &defaults);

        Ok(FileTree {
            root: root.to_path_buf(),
            items,
            gitignore,
            skipped,
        })
    }

    fn scan(
        root: &Path,
        current: &Path,
        depth: usize,
        items: &mut Vec<TreeItem>,
        gitignore: &Option<GitignoreRules>,
        defaults: &[&str],
    ) -> usize {
        let entries = match std::fs::read_dir(current) {
            Ok(e) => e,
            Err(e) => {
                // unreadable dir: skip it, keep the partial tree
                eprintln!("plat: skipping {}: {e}", current.display());
                return 1;
            }
        };

        let mut level: Vec<(String, std::path::PathBuf, bool)> = Vec::new();
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

            level.push((name, path, file_type.is_dir()));
        }

        // this directory's entries: dirs first, both groups alphabetical
        level.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

        let mut skipped = 0usize;
        for (_, path, is_dir) in level {
            items.push(TreeItem { path: path.clone(), is_dir, depth });
            if is_dir && depth + 1 < MAX_DEPTH {
                skipped += Self::scan(root, &path, depth + 1, items, gitignore, defaults);
            }
        }
        skipped
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
    fn build_tree_keeps_children_directly_after_their_dir() {
        let dir = scratch("nested");
        fs::create_dir(dir.join("b_dir")).unwrap();
        fs::create_dir(dir.join("a_dir")).unwrap();
        fs::write(dir.join("z_root.txt"), "").unwrap();
        fs::write(dir.join("b_dir").join("b_child.txt"), "").unwrap();
        fs::write(dir.join("a_dir").join("a_child.txt"), "").unwrap();
        fs::create_dir(dir.join("a_dir").join("deep")).unwrap();
        fs::write(dir.join("a_dir").join("deep").join("deep_child.txt"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        let got = names(&tree);
        // each dir's children follow it, siblings dirs-first then alphabetical
        assert_eq!(
            got,
            vec![
                "a_dir", "deep", "deep_child.txt", "a_child.txt",
                "b_dir", "b_child.txt", "z_root.txt"
            ]
        );

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
    fn build_tree_all_ignored_yields_no_items() {
        // Review Focus: every entry ignored -> empty tree, no panic
        let dir = scratch("allignored");
        fs::write(dir.join(".gitignore"), "*\n").unwrap();
        fs::write(dir.join("file.rs"), "").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("inner.rs"), "").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        assert_eq!(tree.items.len(), 0, "\"*\" ignores everything: {:?}", names(&tree));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_skips_unreadable_dirs_and_keeps_the_rest() {
        // Review Focus: permission denied mid-scan -> partial tree, no panic
        if unsafe { libc::geteuid() } == 0 {
            return; // as root, 0o000 is still readable
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("noperm");
        let blocked = dir.join("no_read");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("hidden.rs"), "").unwrap();
        fs::write(dir.join("visible.rs"), "").unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

        let tree = FileTree::build(&dir).unwrap();
        let got = names(&tree);
        assert!(got.contains(&"visible.rs".to_string()), "scan continues past the unreadable dir");
        assert!(got.contains(&"no_read".to_string()), "the dir itself is still listed");
        assert!(
            tree.items.iter().all(|i| i.path == blocked || !i.path.starts_with(&blocked)),
            "no children of the unreadable dir are listed: {got:?}"
        );

        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_counts_skipped_dirs_for_the_status_bar() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("skipcount");
        let blocked = dir.join("locked");
        fs::create_dir(&blocked).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

        let tree = FileTree::build(&dir).unwrap();
        assert_eq!(tree.skipped, 1, "one unreadable dir is reported");

        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_tree_survives_a_symlink_loop() {
        // Review Focus: a/b/c -> a must not hang or stack-overflow
        #[cfg(unix)]
        {
            let dir = scratch("smlink");
            let a = dir.join("a");
            let b = a.join("b");
            fs::create_dir_all(&b).unwrap();
            std::os::unix::fs::symlink(&a, b.join("c")).unwrap();
            fs::write(dir.join("top.rs"), "").unwrap();

            let tree = FileTree::build(&dir).unwrap();
            let got = names(&tree);
            assert!(got.contains(&"top.rs".to_string()));
            assert!(got.iter().filter(|n| *n == "c").count() <= 1, "loop link listed once at most: {got:?}");
            assert!(tree.items.len() < 100, "no runaway recursion: {} items", tree.items.len());

            fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn is_ignored_returns_true_for_ignored_paths() {        let dir = scratch("isignored");
        fs::write(dir.join(".gitignore"), "*.log\n").unwrap();

        let tree = FileTree::build(&dir).unwrap();
        assert!(tree.is_ignored(&dir.join("file.log")));
        assert!(!tree.is_ignored(&dir.join("file.rs")));

        fs::remove_dir_all(&dir).ok();
    }
}
