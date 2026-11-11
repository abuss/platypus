use crate::gitignore::{GitignoreRules, default_ignores};
use std::path::{Path, PathBuf};
use std::collections::HashSet;

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

/// Ignore-rule context for one tree root (#12). Loaded once per open /
/// re-root / refresh; the lazy tree browser calls `scan_dir` per expanded
/// directory, the stream scan (`ScanJob`) walks everything through it too —
/// one rule set, one scan implementation.
pub struct ScanCtx {
    root: PathBuf,
    gitignore: Option<GitignoreRules>,
    extra: GitignoreRules,
}

impl ScanCtx {
    pub fn new(root: &Path, extra_ignore: &[String]) -> Self {
        ScanCtx {
            root: root.to_path_buf(),
            gitignore: GitignoreRules::from_file(&root.join(".gitignore")).ok(),
            extra: GitignoreRules::from_patterns(extra_ignore),
        }
    }

    /// List and sort one directory whose ITEMS render at `item_depth`
    /// (root's own entries: 0). Returns (items, skipped): skipped counts
    /// unreadable dirs and broken symlinks for the status line.
    pub fn scan_dir(&self, dir: &Path, item_depth: usize) -> (Vec<TreeItem>, usize) {
        if item_depth >= MAX_DEPTH {
            return (Vec::new(), 0); // same cap the old recursion enforced
        }
        let defaults = default_ignores();
        self.scan_level(dir, item_depth, &defaults)
    }

    /// List and sort one directory. Never writes to stderr — scans run while
    /// the TUI owns the terminal (see the 2026-11-05 interleaved-PTY-
    /// corruption incident the original comment recorded).
    fn scan_level(&self, current: &Path, depth: usize, defaults: &[&str]) -> (Vec<TreeItem>, usize) {
        let entries = match std::fs::read_dir(current) {
            Ok(e) => e,
            Err(_) => return (Vec::new(), 1),
        };

        let mut level: Vec<(String, std::path::PathBuf, bool, bool)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
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
            let rel = path.strip_prefix(&self.root).unwrap_or(&path);
            if let Some(rules) = &self.gitignore
                && rules.matches(rel)
            {
                continue;
            }
            if self.extra.matches(rel) {
                continue;
            }

            // Resolve symlinks to determine if they're directories and detect broken links
            let (is_dir, is_broken_symlink) = if file_type.is_symlink() {
                match path.canonicalize() {
                    Ok(canon) => {
                        let is_dir = std::fs::metadata(&canon).map(|m| m.is_dir()).unwrap_or(false);
                        (is_dir, false)
                    }
                    Err(_) => (false, true),
                }
            } else {
                (file_type.is_dir(), false)
            };

            level.push((name, path, is_dir, is_broken_symlink));
        }

        // this directory's entries: dirs first, both groups alphabetical
        level.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

        let mut skipped = 0usize;
        let mut items = Vec::with_capacity(level.len());
        for (_, path, is_dir, is_broken_symlink) in level {
            if is_broken_symlink {
                skipped += 1;
            }
            items.push(TreeItem { path, is_dir, depth });
        }
        (items, skipped)
    }
}

/// One streamed unit: a contiguous run of the final display order (a
/// directory's row, its subtree chunks, then the following file run) — so
/// appended batches are ALWAYS a prefix of what the old full scan produced.
enum ScanMsg {
    Batch(Vec<TreeItem>),
    Done { skipped: usize },
}

/// Full-tree stream, used by `FileTree::build` for the fuzzy picker (#12):
/// the picker ranks matches so it needs every path — it opts into the
/// complete walk via the same `ScanCtx` rules as the lazy browser.
pub struct ScanJob {
    rx: std::sync::mpsc::Receiver<ScanMsg>,
    pub done: bool,
    pub skipped: usize,
}

/// Explicit-stack work item for the scan thread: scan a directory, or send an
/// already-prepared chunk. Replaces the old recursion; pushing a dir's pieces
/// in reverse reproduces the recursive pre-order exactly.
enum ScanFrame {
    Dir(PathBuf, usize, HashSet<PathBuf>),
    Items(Vec<TreeItem>),
}

impl ScanJob {
    pub fn spawn(root: &Path, extra_ignore: &[String]) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = root.to_path_buf();
        let extra_ignore = extra_ignore.to_vec();
        std::thread::spawn(move || {
            let ctx = ScanCtx::new(&root, &extra_ignore);
            let mut skipped = 0usize;
            // (dir to list, depth of its items, canonical paths along the
            // branch — loop guard), mirroring the old recursion's args.
            let mut stack: Vec<ScanFrame> =
                vec![ScanFrame::Dir(root.clone(), 0, HashSet::new())];
            while let Some(frame) = stack.pop() {
                match frame {
                    ScanFrame::Items(v) => {
                        if tx.send(ScanMsg::Batch(v)).is_err() {
                            return; // app stopped listening (re-root/close)
                        }
                    }
                    ScanFrame::Dir(dir, depth, chain) => {
                        let (items, dir_skipped) = ctx.scan_dir(&dir, depth);
                        skipped += dir_skipped;
                        // Split the sorted level into pre-order pieces: a dir
                        // row is followed by its subtree; consecutive files
                        // ride along as one chunk.
                        let mut pieces: Vec<ScanFrame> = Vec::new();
                        let mut files: Vec<TreeItem> = Vec::new();
                        for it in items {
                            if !it.is_dir {
                                files.push(it);
                                continue;
                            }
                            if !files.is_empty() {
                                pieces.push(ScanFrame::Items(std::mem::take(&mut files)));
                            }
                            let mut sub_chain = chain.clone();
                            let descend = depth + 1 < MAX_DEPTH
                                && match it.path.canonicalize() {
                                    // insert()==false: canonical path already on
                                    // this branch — symlink loop, list but don't
                                    // descend (the old guard, same semantics)
                                    Ok(canon) => sub_chain.insert(canon),
                                    Err(_) => false,
                                };
                            // pieces pop in forward order: dir row first,
                            // then its subtree frame, then the next sibling
                            pieces.push(ScanFrame::Items(vec![it.clone()]));
                            if descend {
                                pieces.push(ScanFrame::Dir(it.path, depth + 1, sub_chain));
                            }
                        }
                        if !files.is_empty() {
                            pieces.push(ScanFrame::Items(files));
                        }
                        for piece in pieces.into_iter().rev() {
                            stack.push(piece);
                        }
                    }
                }
            }
            let _ = tx.send(ScanMsg::Done { skipped });
        });
        Self { rx, done: false, skipped: 0 }
    }

    /// Nonblocking drain of up to `max` batches, flattened. Second element =
    /// stream ended (Done received or thread gone). `skipped` is final after.
    pub fn drain(&mut self, max: usize) -> (Vec<TreeItem>, bool) {
        let mut out = Vec::new();
        use std::sync::mpsc::TryRecvError;
        for _ in 0..max {
            match self.rx.try_recv() {
                Ok(ScanMsg::Batch(v)) => out.extend(v),
                Ok(ScanMsg::Done { skipped }) => {
                    self.skipped = skipped;
                    self.done = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.done = true;
                    break;
                }
            }
        }
        (out, self.done)
    }

}

impl FileTree {
    /// Full synchronous scan — the fuzzy picker needs every path at once, so
    /// it opts into the complete walk (which now shares the stream machinery
    /// and therefore the exact same order and semantics).
    pub fn build(root: &Path, extra_ignore: &[String]) -> Result<Self, String> {
        let gitignore = GitignoreRules::from_file(&root.join(".gitignore")).ok();
        let job = ScanJob::spawn(root, extra_ignore);
        let mut items = Vec::new();
        while let Ok(msg) = job.rx.recv() {
            match msg {
                ScanMsg::Batch(v) => items.extend(v),
                ScanMsg::Done { skipped } => {
                    return Ok(FileTree {
                        root: root.to_path_buf(),
                        items,
                        gitignore,
                        skipped,
                    });
                }
            }
        }
        Err("tree scan died".into())
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
    fn scan_job_streams_the_same_items_as_build_in_the_same_order() {
        // #12: the streaming scanner must be order- and content-identical to
        // the old full scan, batch boundaries aside. Include nested dirs, a
        // gitignored entry, and a symlink loop for full coverage.
        let dir = scratch("stream");
        fs::create_dir(dir.join("b_dir")).unwrap();
        fs::create_dir(dir.join("a_dir")).unwrap();
        fs::create_dir(dir.join("a_dir").join("deep")).unwrap();
        fs::write(dir.join("a_dir").join("deep").join("deep_child.txt"), "").unwrap();
        fs::write(dir.join("a_dir").join("a_child.txt"), "").unwrap();
        fs::write(dir.join("b_dir").join("b_child.txt"), "").unwrap();
        fs::write(dir.join("z_root.txt"), "").unwrap();
        fs::write(dir.join("skip.log"), "").unwrap();
        fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        let a = dir.join("a_dir");
        std::os::unix::fs::symlink(&a, dir.join("b_dir").join("loop_back")).unwrap();

        let want = FileTree::build(&dir, &[]).unwrap();
        let mut job = ScanJob::spawn(&dir, &[]);
        let mut got: Vec<TreeItem> = Vec::new();
        let mut done = false;
        for _ in 0..10_000 {
            let (batch, ended) = job.drain(1); // tiny batches: exercise multi-pump streaming
            got.extend(batch);
            if ended {
                done = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(done, "scan stream must terminate");
        let key = |v: &Vec<TreeItem>| -> Vec<(String, bool, usize)> {
            v.iter()
                .map(|i| {
                    (
                        i.path.strip_prefix(&dir).unwrap().to_string_lossy().into_owned(),
                        i.is_dir,
                        i.depth,
                    )
                })
                .collect()
        };
        assert_eq!(key(&got), key(&want.items), "streamed order == full-scan order");
        assert_eq!(job.skipped, want.skipped, "skipped counts match");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_job_unreadable_root_yields_empty_done() {
        // build() has always tolerated this; the stream must too.
        let dir = scratch("noexist");
        fs::remove_dir(&dir).unwrap();
        let mut job = ScanJob::spawn(&dir, &[]);
        let (batch, ended) = job.drain(64);
        assert!(batch.is_empty());
        let _ = ended; // may or may not land on first drain; pump until done:
        for _ in 0..1000 {
            let (b, e) = job.drain(64);
            assert!(b.is_empty());
            if e {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn build_tree_on_simple_structure() {
        let dir = scratch("simple");
        fs::create_dir(dir.join("subdir")).unwrap();
        fs::write(dir.join("file1.txt"), "").unwrap();
        fs::write(dir.join("subdir").join("file2.txt"), "").unwrap();

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
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

            let tree = FileTree::build(&dir, &[]).unwrap();
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

        let tree = FileTree::build(&dir, &[]).unwrap();
        assert!(tree.is_ignored(&dir.join("file.log")));
        assert!(!tree.is_ignored(&dir.join("file.rs")));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn symlinked_dir_is_listed_and_traversed() {
        #[cfg(unix)]
        {
            let dir = scratch("symdir");
            let link_target = dir.join("target");
            fs::create_dir(&link_target).unwrap();
            fs::write(link_target.join("file_in_target.txt"), "").unwrap();
            std::os::unix::fs::symlink(&link_target, dir.join("link")).unwrap();

            let tree = FileTree::build(&dir, &[]).unwrap();
            // Check that link/ appears as a directory entry
            assert!(tree.items.iter().any(|i| i.path.ends_with("link") && i.is_dir), 
                    "symlinked dir appears with is_dir=true");
            // Check that file_in_target.txt appears under link/
            assert!(tree.items.iter().any(|i| i.path.ends_with("link/file_in_target.txt") || i.path.ends_with("link") && i.path.parent().map_or(false, |p| p.ends_with("link")) || 
                                               i.path.starts_with(&dir.join("link"))), 
                    "file under symlinked dir is traversed: paths = {:?}", tree.items.iter().map(|i| &i.path).collect::<Vec<_>>());

            fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn self_referencing_symlink_terminates() {
        #[cfg(unix)]
        {
            let dir = scratch("selfref");
            let a = dir.join("a");
            fs::create_dir(&a).unwrap();
            std::os::unix::fs::symlink(&a, a.join("link")).unwrap();

            let tree = FileTree::build(&dir, &[]).unwrap();
            // must not hang and must not have runaway recursion
            assert!(tree.items.len() < 1000, "bounded by MAX_DEPTH: {} items", tree.items.len());
            let got = names(&tree);
            assert!(got.contains(&"link".to_string()), "self-referencing link is listed");

            fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn unresolvable_symlink_target_counts_as_skipped() {
        #[cfg(unix)]
        {
            let dir = scratch("dangling");
            let real = dir.join("real");
            fs::create_dir(&real).unwrap();
            // create a symlink to real, then delete real
            std::os::unix::fs::symlink(&real, dir.join("link_to_gone")).unwrap();
            fs::remove_dir(&real).unwrap();

            let tree = FileTree::build(&dir, &[]).unwrap();
            let got = names(&tree);
            // dangling symlink is still listed (as is_dir=false)
            assert!(got.contains(&"link_to_gone".to_string()), "dangling symlink is listed: {got:?}");
            // and increments skipped counter since canonicalize fails on a dir symlink
            assert!(tree.skipped > 0, "canonicalize failure increments skipped: {} items", tree.skipped);

            fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn config_ignore_patterns_are_skipped_like_gitignore_rules() {
        let dir = scratch("config_ignore");
        fs::create_dir(dir.join("dist")).unwrap();
        fs::create_dir(dir.join("vendor")).unwrap();
        fs::write(dir.join("vendor").join("lib.min.js"), "").unwrap();
        fs::create_dir(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("keep.rs"), "").unwrap();

        let tree = FileTree::build(&dir, &["dist".into(), "*.min.js".into()]).unwrap();
        let paths: Vec<_> = tree.items.iter().map(|i| i.path.strip_prefix(&dir).unwrap().to_path_buf()).collect();
        assert!(!paths.iter().any(|p| p == Path::new("dist")), "dist dir is ignored");
        assert!(!paths.iter().any(|p| p.to_string_lossy().ends_with(".min.js")), "*.min.js files are ignored");
        assert!(paths.iter().any(|p| p == Path::new("src/keep.rs")), "non-matching files are kept");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn built_in_defaults_still_skipped_with_empty_extra() {
        let dir = scratch("defaults_empty");
        fs::create_dir(dir.join("node_modules")).unwrap();
        fs::create_dir(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("main.rs"), "").unwrap();

        let tree = FileTree::build(&dir, &[]).unwrap();
        let got = names(&tree);
        assert!(!got.contains(&"node_modules".to_string()), "built-in defaults still ignored");
        assert!(got.contains(&"src".to_string()), "other dirs are still present");

        fs::remove_dir_all(&dir).ok();
    }
}
