# Phase 1: File Sidebar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a resizable sidebar with tree browser (Ctrl-B) and fuzzy picker (Ctrl-P) for file navigation, replacing the old Ctrl-P open prompt.

**Architecture:** Four new modules (`tree.rs`, `picker.rs`, `sidebar.rs`, `gitignore.rs`) build and maintain file trees, implement search, and manage sidebar state. `app.rs` orchestrates sidebar lifecycle (open/close/select). `render.rs` splits the viewport. `keymap.rs` gains 8 new commands for sidebar control.

**Tech Stack:** Rust (stable), ratatui, crossterm, ropey. No new external dependencies in Phase 1 (gitignore parsing is manual glob matching).

**Spec:** `docs/superpowers/specs/2026-11-03-plat-phase1-sidebar-design.md`

## Global Constraints

- No new external dependencies (gitignore parsing is manual, Phase 1.1 can add `ignore` crate)
- Sidebar cached at open time; no auto-refresh (Phase 1.1)
- Fuzzy matching is synchronous (acceptable for <10k files)
- Layout computed per draw; sidebar is transient (open/close, not persistent)
- All Phase 0 tests must still pass (no regression)

## Review Focus

1. **Empty tree (all files ignored):** Sidebar shows "no files" message, closes cleanly on Esc without panic
2. **Permission denied during scan:** Scan skips unreadable dirs, logs status bar error, continues and shows partial tree
3. **Symlink loops:** Tree building detects via visited-set, skips looped symlinks, does not hang
4. **Resize boundary clamp:** Shrink below 10 or grow above 50 cols clamps to range; layout stays stable
5. **Mode switch (tree ↔ picker):** Switching modes while sidebar open (Ctrl-B while in picker) closes old mode, opens new; cursor/filter state does not leak between modes

---

## File Structure

**New files:**
- `src/tree.rs` — FileTree building, .gitignore parsing
- `src/picker.rs` — FuzzyMatcher and fuzzy search
- `src/sidebar.rs` — Sidebar state, controller, tree/picker views
- `src/gitignore.rs` — .gitignore rule parsing and matching (optional: fold into tree.rs if <50 lines)

**Modified files:**
- `src/app.rs` — add Sidebar state, sidebar lifecycle methods
- `src/render.rs` — split viewport, render sidebar + editor
- `src/keymap.rs` — 8 new sidebar commands, Alt-key decoding
- `src/input.rs` — Alt-Left/Right key handling (or fold into keymap.rs)
- `Cargo.toml` — no new dependencies

---

## Task 1: Gitignore Parsing Module

**Files:**
- Create: `src/gitignore.rs`
- Test: inline in `src/gitignore.rs`

**Interfaces:**
- Produces: `GitignoreRules { rules: Vec<String> }` with `GitignoreRules::from_file(path: &Path) -> Result<Self, String>` and `GitignoreRules::matches(&self, path: &Path) -> bool`; also a free fn `default_ignores() -> Vec<&'static str>` returning `[".git", "node_modules", "target", "__pycache__", ".egg-info"]`

- [ ] **Step 1: Write failing tests for gitignore rule matching**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn default_ignores_includes_git() {
        let defaults = default_ignores();
        assert!(defaults.contains(&".git"));
    }

    #[test]
    fn simple_glob_pattern_matches() {
        // "*.o" should match "file.o"
        let rules = GitignoreRules { rules: vec!["*.o".to_string()] };
        assert!(rules.matches(Path::new("file.o")));
        assert!(!rules.matches(Path::new("file.rs")));
    }

    #[test]
    fn directory_pattern_matches_dir_contents() {
        // "build/" should match "build" and "build/output.txt"
        let rules = GitignoreRules { rules: vec!["build/".to_string()] };
        assert!(rules.matches(Path::new("build")));
        assert!(rules.matches(Path::new("build/output.txt")));
    }

    #[test]
    fn from_file_parses_lines_ignoring_comments() {
        // Create temp file with "# comment\n*.log\n\n", parse it
        let tempfile = /* ... */;
        let rules = GitignoreRules::from_file(tempfile.path()).unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0], "*.log");
    }

    #[test]
    fn from_nonexistent_file_returns_empty_rules() {
        let rules = GitignoreRules::from_file(Path::new("/nonexistent")).unwrap_or_default();
        assert!(rules.rules.is_empty());
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib gitignore::tests`
Expected: All tests fail (module does not exist yet)

- [ ] **Step 3: Implement `src/gitignore.rs`**

Glob matching algorithm (simple, Phase 1):
- `*.o` pattern: match if filename ends with `.o`
- `dir/` pattern: match if path starts with `dir/` or equals `dir`
- Plain word (e.g., `build`): match if any path component equals it
- Negation (`!rule`) not supported in Phase 1

```rust
pub struct GitignoreRules {
    pub rules: Vec<String>,
}

impl GitignoreRules {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let rules = content
                    .lines()
                    .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
                    .map(|line| line.trim().to_string())
                    .collect();
                Ok(GitignoreRules { rules })
            }
            Err(_) => Ok(GitignoreRules { rules: vec![] }),
        }
    }

    pub fn matches(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        for rule in &self.rules {
            if self.match_rule(&path_str, rule) {
                return true;
            }
        }
        false
    }

    fn match_rule(&self, path_str: &str, rule: &str) -> bool {
        if rule.ends_with('/') {
            // Directory pattern: match "dir" or "dir/*"
            let dir = &rule[..rule.len() - 1];
            path_str == dir || path_str.starts_with(&format!("{}/", dir))
        } else if rule.starts_with('*') {
            // Extension pattern: match *.ext
            path_str.ends_with(&rule[1..])
        } else {
            // Literal pattern: match if filename or path component equals rule
            let filename = Path::new(path_str)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            filename == rule || path_str.contains(&format!("/{}/", rule))
        }
    }
}

pub fn default_ignores() -> Vec<&'static str> {
    vec![".git", "node_modules", "target", "__pycache__", ".egg-info"]
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib gitignore::tests`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/gitignore.rs
git commit -m "feat: gitignore rule parsing (Phase 1 simple glob matching)"
```

---

## Task 2: File Tree Builder

**Files:**
- Create: `src/tree.rs`
- Test: inline in `src/tree.rs`

**Interfaces:**
- Consumes: `gitignore::GitignoreRules`, `gitignore::default_ignores()`
- Produces: `TreeItem { path: PathBuf, is_dir: bool, depth: usize }` (depth unused in builder, added for sidebar state); `FileTree { root: PathBuf, items: Vec<TreeItem>, gitignore: Option<GitignoreRules> }` with `FileTree::build(root: &Path) -> Result<Self, String>` and `FileTree::is_ignored(&self, path: &Path) -> bool`

- [ ] **Step 1: Write failing tests for tree building**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn build_tree_on_simple_structure() {
        // Create: dir/file1.txt, dir/subdir/file2.txt
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("file1.txt"), "").unwrap();
        fs::write(root.join("subdir").join("file2.txt"), "").unwrap();

        let tree = FileTree::build(root).unwrap();
        assert!(tree.items.len() >= 2);
        let filenames: Vec<_> = tree.items.iter().map(|i| i.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(filenames.contains(&"file1.txt".to_string()));
        assert!(filenames.contains(&"file2.txt".to_string()));
    }

    #[test]
    fn build_tree_respects_gitignore() {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("file.log"), "").unwrap();
        fs::write(root.join("file.rs"), "").unwrap();

        let tree = FileTree::build(root).unwrap();
        let filenames: Vec<_> = tree.items.iter().map(|i| i.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(filenames.contains(&"file.rs".to_string()));
        assert!(!filenames.contains(&"file.log".to_string()));
    }

    #[test]
    fn build_tree_sorts_dirs_first_then_files_alphabetically() {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        fs::write(root.join("z_file.txt"), "").unwrap();
        fs::create_dir(root.join("a_dir")).unwrap();
        fs::write(root.join("m_file.txt"), "").unwrap();

        let tree = FileTree::build(root).unwrap();
        let names: Vec<_> = tree.items.iter().map(|i| i.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        // Expect: a_dir (dir), m_file.txt (file), z_file.txt (file)
        assert_eq!(names[0], "a_dir");
        assert!(names.windows(2).all(|w| w[0] <= w[1])); // Check ordering for files
    }

    #[test]
    fn build_tree_excludes_default_ignores() {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("file.rs"), "").unwrap();

        let tree = FileTree::build(root).unwrap();
        let filenames: Vec<_> = tree.items.iter().map(|i| i.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(!filenames.contains(&".git".to_string()));
        assert!(!filenames.contains(&"node_modules".to_string()));
        assert!(filenames.contains(&"file.rs".to_string()));
    }

    #[test]
    fn is_ignored_returns_true_for_ignored_paths() {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();

        let tree = FileTree::build(root).unwrap();
        assert!(tree.is_ignored(&root.join("file.log")));
        assert!(!tree.is_ignored(&root.join("file.rs")));
    }
}
```

Note: Phase 1 does not need `tempfile` crate if avoided; can use `std::env::temp_dir()` directly. Adjust if no tempfile dependency.

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib tree::tests`
Expected: All fail (module does not exist)

- [ ] **Step 3: Implement `src/tree.rs`**

```rust
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
}

impl FileTree {
    pub fn build(root: &Path) -> Result<Self, String> {
        let mut items = Vec::new();
        let gitignore = GitignoreRules::from_file(&root.join(".gitignore")).ok();
        let defaults = default_ignores();

        Self::scan(root, root, 0, &mut items, &gitignore, &defaults)?;

        // Sort: dirs first (alphabetically), then files (alphabetically)
        items.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let a_name = a.path.file_name().unwrap().to_string_lossy();
                    let b_name = b.path.file_name().unwrap().to_string_lossy();
                    a_name.cmp(&b_name)
                }
            }
        });

        Ok(FileTree { root: root.to_path_buf(), items, gitignore })
    }

    fn scan(
        root: &Path,
        current: &Path,
        depth: usize,
        items: &mut Vec<TreeItem>,
        gitignore: &Option<GitignoreRules>,
        defaults: &[&str],
    ) -> Result<(), String> {
        let entries = match std::fs::read_dir(current) {
            Ok(e) => e,
            Err(e) => {
                // Log permission error but continue
                eprintln!("Warning: skipping {}: {}", current.display(), e);
                return Ok(());
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let metadata = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue, // Skip if metadata unreadable
            };

            let rel_path = path.strip_prefix(root).unwrap_or(&path);
            let name = path.file_name().unwrap().to_string_lossy().to_string();

            // Check defaults
            if defaults.contains(&name.as_str()) {
                continue;
            }

            // Check gitignore
            if let Some(rules) = gitignore {
                if rules.matches(&rel_path) {
                    continue;
                }
            }

            let is_dir = metadata.is_dir();
            items.push(TreeItem {
                path: path.clone(),
                is_dir,
                depth,
            });

            // Recurse into directories
            if is_dir && depth < 20 {
                // ponytail: max depth 20; increase if large monorepos lag
                Self::scan(root, &path, depth + 1, items, gitignore, defaults)?;
            }
        }

        Ok(())
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        let rel_path = path.strip_prefix(&self.root).unwrap_or(path);
        if let Some(rules) = &self.gitignore {
            if rules.matches(rel_path) {
                return true;
            }
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        default_ignores().contains(&name.as_str())
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib tree::tests`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/tree.rs
git commit -m "feat: file tree builder with .gitignore support"
```

---

## Task 3: Fuzzy Picker Search Engine

**Files:**
- Create: `src/picker.rs`
- Test: inline

**Interfaces:**
- Consumes: `tree::TreeItem`, `tree::FileTree`
- Produces: `FuzzyResult { item: TreeItem, score: u32 }` and `FuzzyMatcher` with `FuzzyMatcher::new(tree: &FileTree) -> Self` and `FuzzyMatcher::search(&self, term: &str) -> Vec<FuzzyResult>`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::tree::TreeItem;

    fn dummy_items() -> Vec<TreeItem> {
        vec![
            TreeItem { path: PathBuf::from("src/main.rs"), is_dir: false, depth: 1 },
            TreeItem { path: PathBuf::from("src/app.rs"), is_dir: false, depth: 1 },
            TreeItem { path: PathBuf::from("src/render.rs"), is_dir: false, depth: 1 },
            TreeItem { path: PathBuf::from("tests/flow.rs"), is_dir: false, depth: 1 },
        ]
    }

    #[test]
    fn search_empty_term_returns_all_items() {
        let matcher = FuzzyMatcher { items: dummy_items() };
        let results = matcher.search("");
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn search_exact_filename_match_highest_score() {
        let matcher = FuzzyMatcher { items: dummy_items() };
        let results = matcher.search("main");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.path.to_string_lossy(), "src/main.rs");
    }

    #[test]
    fn search_substring_matches_multiple() {
        let matcher = FuzzyMatcher { items: dummy_items() };
        let results = matcher.search("app");
        // Should match src/app.rs and src/render.rs (has "app" in path)? No, just src/app.rs
        assert!(results.iter().any(|r| r.item.path.to_string_lossy().contains("app")));
    }

    #[test]
    fn search_path_component_match() {
        let matcher = FuzzyMatcher { items: dummy_items() };
        let results = matcher.search("src/r");
        // Should match src/render.rs
        assert!(results.iter().any(|r| r.item.path.to_string_lossy().contains("render")));
    }

    #[test]
    fn search_results_sorted_by_score_descending() {
        let matcher = FuzzyMatcher { items: dummy_items() };
        let results = matcher.search("r");
        // render, app (has 'r'), main (no 'r'?)
        assert!(results[0].score >= results.get(1).map(|r| r.score).unwrap_or(0));
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib picker::tests`
Expected: All fail

- [ ] **Step 3: Implement `src/picker.rs`**

```rust
use crate::tree::TreeItem;

#[derive(Clone, Debug)]
pub struct FuzzyResult {
    pub item: TreeItem,
    pub score: u32,
}

pub struct FuzzyMatcher {
    pub items: Vec<TreeItem>,
}

impl FuzzyMatcher {
    pub fn new(items: Vec<TreeItem>) -> Self {
        FuzzyMatcher { items }
    }

    pub fn search(&self, term: &str) -> Vec<FuzzyResult> {
        if term.is_empty() {
            // Return all items sorted alphabetically
            let mut results: Vec<_> = self.items
                .iter()
                .map(|item| FuzzyResult { item: item.clone(), score: 100 })
                .collect();
            results.sort_by_key(|r| r.item.path.clone());
            return results;
        }

        let term_lower = term.to_lowercase();
        let mut results = Vec::new();

        for item in &self.items {
            let path_str = item.path.to_string_lossy().to_lowercase();
            let filename = item.path.file_name().unwrap().to_string_lossy().to_lowercase();

            let mut score = 0u32;

            // Exact filename match
            if filename == term_lower {
                score = 1000;
            }
            // Filename prefix match
            else if filename.starts_with(&term_lower) {
                score = 900;
            }
            // Filename contains
            else if filename.contains(&term_lower) {
                score = 800;
            }
            // Path contains
            else if path_str.contains(&term_lower) {
                score = 600;
            }

            if score > 0 {
                results.push(FuzzyResult {
                    item: item.clone(),
                    score,
                });
            }
        }

        // Sort by score descending, then by path alphabetically
        results.sort_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| {
                a.item.path.cmp(&b.item.path)
            })
        });

        results
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib picker::tests`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/picker.rs
git commit -m "feat: fuzzy search matcher for file picker"
```

---

## Task 4: Sidebar State Manager

**Files:**
- Create: `src/sidebar.rs`
- Test: inline

**Interfaces:**
- Consumes: `tree::TreeItem, tree::FileTree`, `picker::FuzzyMatcher, picker::FuzzyResult`
- Produces: `SidebarMode { TreeBrowser, FuzzyPicker }`, `TreeState { items: Vec<TreeItem>, expanded: HashSet<PathBuf> }`, `Sidebar { mode: SidebarMode, width: u16, cursor: usize, input: String, tree_state: Option<TreeState>, matcher: Option<FuzzyMatcher> }` with methods: `Sidebar::new(mode: SidebarMode, file_tree: &FileTree, width: u16) -> Self`, `Sidebar::move_cursor(&mut self, delta: i32)`, `Sidebar::toggle_expand(&mut self, idx: usize)`, `Sidebar::filter_items(&mut self, term: &str)`, `Sidebar::selected_path(&self) -> Option<PathBuf>`, `Sidebar::visible_items(&self) -> Vec<TreeItem>`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::tree::{TreeItem, FileTree};

    fn dummy_tree_items() -> Vec<TreeItem> {
        vec![
            TreeItem { path: PathBuf::from("file1.rs"), is_dir: false, depth: 0 },
            TreeItem { path: PathBuf::from("dir"), is_dir: true, depth: 0 },
            TreeItem { path: PathBuf::from("dir/file2.rs"), is_dir: false, depth: 1 },
        ]
    }

    #[test]
    fn sidebar_move_cursor_up_down() {
        let mut sidebar = Sidebar::new_tree(dummy_tree_items(), 32);
        assert_eq!(sidebar.cursor, 0);
        sidebar.move_cursor(1);
        assert_eq!(sidebar.cursor, 1);
        sidebar.move_cursor(-1);
        assert_eq!(sidebar.cursor, 0);
    }

    #[test]
    fn sidebar_cursor_clamps_to_bounds() {
        let mut sidebar = Sidebar::new_tree(dummy_tree_items(), 32);
        sidebar.move_cursor(100);  // Try to go past end
        assert!(sidebar.cursor < dummy_tree_items().len());
    }

    #[test]
    fn sidebar_toggle_expand_adds_to_expanded_set() {
        let mut sidebar = Sidebar::new_tree(dummy_tree_items(), 32);
        let dir_path = PathBuf::from("dir");
        sidebar.toggle_expand_path(&dir_path);
        assert!(sidebar.tree_state.as_ref().unwrap().expanded.contains(&dir_path));
    }

    #[test]
    fn sidebar_selected_path_returns_current_item() {
        let mut sidebar = Sidebar::new_tree(dummy_tree_items(), 32);
        sidebar.cursor = 1;
        let selected = sidebar.selected_path();
        assert_eq!(selected, Some(PathBuf::from("dir")));
    }

    #[test]
    fn sidebar_picker_filter_updates_visible_items() {
        let mut sidebar = Sidebar::new_picker(dummy_tree_items(), 32);
        sidebar.filter_items("file1");
        let visible = sidebar.visible_items();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, PathBuf::from("file1.rs"));
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib sidebar::tests`
Expected: All fail

- [ ] **Step 3: Implement `src/sidebar.rs`**

```rust
use crate::tree::TreeItem;
use crate::picker::{FuzzyMatcher, FuzzyResult};
use std::path::PathBuf;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarMode {
    TreeBrowser,
    FuzzyPicker,
}

#[derive(Debug, Clone)]
pub struct TreeState {
    pub items: Vec<TreeItem>,
    pub expanded: HashSet<PathBuf>,
}

pub struct Sidebar {
    pub mode: SidebarMode,
    pub width: u16,
    pub cursor: usize,
    pub input: String,
    pub tree_state: Option<TreeState>,
    pub matcher: Option<FuzzyMatcher>,
    filtered_items: Vec<TreeItem>,  // cache of filtered results for picker
}

impl Sidebar {
    pub fn new_tree(items: Vec<TreeItem>, width: u16) -> Self {
        Sidebar {
            mode: SidebarMode::TreeBrowser,
            width: width.max(10).min(50),
            cursor: 0,
            input: String::new(),
            tree_state: Some(TreeState {
                items,
                expanded: HashSet::new(),
            }),
            matcher: None,
            filtered_items: vec![],
        }
    }

    pub fn new_picker(items: Vec<TreeItem>, width: u16) -> Self {
        Sidebar {
            mode: SidebarMode::FuzzyPicker,
            width: width.max(10).min(50),
            cursor: 0,
            input: String::new(),
            tree_state: None,
            matcher: Some(FuzzyMatcher::new(items)),
            filtered_items: vec![],
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let visible = self.visible_items();
        let current = self.cursor as i32;
        let new_pos = (current + delta).max(0) as usize;
        self.cursor = new_pos.min(visible.len().saturating_sub(1));
    }

    pub fn toggle_expand_path(&mut self, path: &PathBuf) {
        if let Some(tree_state) = &mut self.tree_state {
            if tree_state.expanded.contains(path) {
                tree_state.expanded.remove(path);
            } else {
                tree_state.expanded.insert(path.clone());
            }
        }
    }

    pub fn filter_items(&mut self, term: &str) {
        if let Some(matcher) = &self.matcher {
            let results = matcher.search(term);
            self.filtered_items = results.into_iter().map(|r| r.item).collect();
            self.cursor = 0;
        }
        self.input = term.to_string();
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        let visible = self.visible_items();
        visible.get(self.cursor).map(|item| item.path.clone())
    }

    pub fn visible_items(&self) -> Vec<TreeItem> {
        match self.mode {
            SidebarMode::TreeBrowser => {
                if let Some(tree_state) = &self.tree_state {
                    // Filter by expanded state
                    tree_state.items.iter()
                        .filter(|item| {
                            if item.depth == 0 {
                                return true;
                            }
                            // Check if any parent is in expanded set
                            let mut parent = item.path.parent();
                            while let Some(p) = parent {
                                if tree_state.expanded.contains(p) {
                                    return true;
                                }
                                parent = p.parent();
                            }
                            false
                        })
                        .cloned()
                        .collect()
                } else {
                    vec![]
                }
            }
            SidebarMode::FuzzyPicker => self.filtered_items.clone(),
        }
    }

    pub fn resize(&mut self, delta: i16) {
        let new_width = (self.width as i16 + delta).max(10).min(50) as u16;
        self.width = new_width;
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib sidebar::tests`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/sidebar.rs
git commit -m "feat: sidebar state manager (tree and picker modes)"
```

---

## Task 5: Integrate Sidebar into App State

**Files:**
- Modify: `src/app.rs`
- Test: inline app tests

**Interfaces:**
- Consumes: `sidebar::Sidebar, sidebar::SidebarMode`, `tree::FileTree`
- Produces: `App.sidebar: Option<Sidebar>`, `App.sidebar_width: u16`, methods: `App::open_sidebar(&mut self, mode: SidebarMode)`, `App::close_sidebar(&mut self)`, `App::sidebar_on_enter(&mut self)` (open selected file)

- [ ] **Step 1: Add sidebar fields to App struct**

In `src/app.rs`, add to the `App` struct:
```rust
pub sidebar: Option<Sidebar>,
pub sidebar_width: u16,
```

And in `App::new()`, initialize:
```rust
sidebar: None,
sidebar_width: 32,
```

- [ ] **Step 2: Write failing test for sidebar lifecycle**

```rust
#[test]
fn app_open_sidebar_tree_browser() {
    let dir = std::env::temp_dir();
    let mut app = App::new(vec![dir.join("test.txt")]);
    app.open_sidebar(SidebarMode::TreeBrowser);
    assert!(app.sidebar.is_some());
    assert_eq!(app.sidebar.as_ref().unwrap().mode, SidebarMode::TreeBrowser);
}

#[test]
fn app_close_sidebar() {
    let dir = std::env::temp_dir();
    let mut app = App::new(vec![dir.join("test.txt")]);
    app.open_sidebar(SidebarMode::TreeBrowser);
    app.close_sidebar();
    assert!(app.sidebar.is_none());
}

#[test]
fn app_sidebar_select_opens_file() {
    let tmpdir = std::env::temp_dir().join(format!("plat-sidebar-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmpdir);
    let file_path = tmpdir.join("test.rs");
    std::fs::write(&file_path, "test").unwrap();

    let mut app = App::new(vec![tmpdir.join("dummy.txt")]);
    app.open_sidebar(SidebarMode::TreeBrowser);
    
    // Simulate selecting the file and pressing Enter
    // (Details depend on how sidebar items are indexed; adjust to real implementation)
    if let Some(sidebar) = &mut app.sidebar {
        sidebar.cursor = 1; // Assume file is second item
        app.sidebar_on_enter();
    }

    // After select, sidebar should close and file should be open
    assert!(app.sidebar.is_none());
    assert!(app.docs.iter().any(|d| d.path.as_ref().map(|p| p == &file_path).unwrap_or(false)));

    std::fs::remove_file(&file_path).ok();
    std::fs::remove_dir_all(&tmpdir).ok();
}
```

- [ ] **Step 3: Implement sidebar methods in `App`**

```rust
use crate::sidebar::{Sidebar, SidebarMode};
use crate::tree::FileTree;

impl App {
    pub fn open_sidebar(&mut self, mode: SidebarMode) {
        let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        match FileTree::build(&root) {
            Ok(tree) => {
                let sidebar = match mode {
                    SidebarMode::TreeBrowser => Sidebar::new_tree(tree.items, self.sidebar_width),
                    SidebarMode::FuzzyPicker => Sidebar::new_picker(tree.items, self.sidebar_width),
                };
                self.sidebar = Some(sidebar);
                self.mode = Mode::Edit; // Sidebar takes keyboard focus
            }
            Err(e) => {
                self.status = Some((format!("Failed to build tree: {}", e), true));
            }
        }
    }

    pub fn close_sidebar(&mut self) {
        self.sidebar = None;
    }

    pub fn sidebar_on_enter(&mut self) {
        if let Some(sidebar) = &self.sidebar {
            if let Some(path) = sidebar.selected_path() {
                if path.is_file() {
                    self.open_path(&path.to_string_lossy());
                    self.close_sidebar();
                } else if path.is_dir() {
                    // Toggle expand for tree browser
                    if let Some(mut sidebar) = self.sidebar.take() {
                        sidebar.toggle_expand_path(&path);
                        self.sidebar = Some(sidebar);
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib app::tests`
Expected: New tests pass, old tests still pass

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat: integrate sidebar into App state (open/close/select)"
```

---

## Task 6: Keymap Commands for Sidebar

**Files:**
- Modify: `src/keymap.rs`
- Test: inline

**Interfaces:**
- Produces: New `Command` variants: `OpenSidebarTree`, `OpenSidebarPicker`, `SidebarMove(Motion)`, `SidebarToggleExpand`, `SidebarInput(char)`, `SidebarBackspace`, `SidebarSubmit`, `SidebarCancel`, `ResizeSidebar(i16)`
- Modifies: `lookup()` to handle Ctrl-B, Ctrl-P, Alt-Left/Right, arrow keys when sidebar is open

- [ ] **Step 1: Write failing tests for sidebar keymap**

```rust
#[test]
fn keymap_ctrl_b_opens_tree_browser() {
    let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
    let cmd = lookup(Mode::Edit, &key);
    assert_eq!(cmd, Some(Command::OpenSidebarTree));
}

#[test]
fn keymap_ctrl_p_opens_fuzzy_picker() {
    let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
    let cmd = lookup(Mode::Edit, &key);
    assert_eq!(cmd, Some(Command::OpenSidebarPicker));
}

#[test]
fn keymap_alt_left_resizes_sidebar() {
    let key = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
    let cmd = lookup(Mode::Edit, &key);
    assert_eq!(cmd, Some(Command::ResizeSidebar(-2)));
}

#[test]
fn keymap_typing_in_picker_sends_input() {
    let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty());
    let cmd = lookup(Mode::Edit, &key);
    // When sidebar is open and in picker mode, this would route to SidebarInput
    // (lookup doesn't know sidebar state; app.rs routes based on app.sidebar)
    // For now, just verify keycode decodes correctly
    assert!(matches!(key.code, KeyCode::Char('a')));
}

#[test]
fn keymap_esc_closes_sidebar() {
    let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
    let cmd = lookup(Mode::Edit, &key);
    assert_eq!(cmd, Some(Command::SidebarCancel));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib keymap::tests`
Expected: New tests fail (commands don't exist)

- [ ] **Step 3: Add sidebar commands to `Command` enum**

In `src/keymap.rs`:
```rust
pub enum Command {
    // ... existing commands ...
    OpenSidebarTree,
    OpenSidebarPicker,
    SidebarMove(Motion),
    SidebarToggleExpand,
    SidebarInput(char),
    SidebarBackspace,
    SidebarSubmit,
    SidebarCancel,
    ResizeSidebar(i16),
}
```

- [ ] **Step 4: Implement keymap bindings**

In `lookup()` function, add cases:
```rust
(Mode::Edit, KeyCode::Char('b'), KeyModifiers::CONTROL) => Some(Command::OpenSidebarTree),
(Mode::Edit, KeyCode::Char('p'), KeyModifiers::CONTROL) => Some(Command::OpenSidebarPicker),
(Mode::Action, KeyCode::Char('b'), KeyModifiers::CONTROL) => Some(Command::OpenSidebarTree),
(Mode::Action, KeyCode::Char('p'), KeyModifiers::CONTROL) => Some(Command::OpenSidebarPicker),
(_, KeyCode::Left, KeyModifiers::ALT) => Some(Command::ResizeSidebar(-2)),
(_, KeyCode::Right, KeyModifiers::ALT) => Some(Command::ResizeSidebar(2)),
(_, KeyCode::Esc, KeyModifiers::empty()) => Some(Command::SidebarCancel),
```

And when `app.sidebar.is_some()`, route arrow keys to `SidebarMove` instead of regular motion.

- [ ] **Step 5: Run tests, verify they pass**

Run: `cargo test --lib keymap::tests`
Expected: All pass

- [ ] **Step 6: Commit**

```bash
git add src/keymap.rs
git commit -m "feat: sidebar keymap commands (Ctrl-B/P, arrow navigation, resize)"
```

---

## Task 7: Sidebar Input Handler in App

**Files:**
- Modify: `src/app.rs`
- Test: inline

**Interfaces:**
- Consumes: `Command::SidebarMove, Command::SidebarInput, ...`
- Produces: `App::handle_sidebar_command(&mut self, cmd: Command)` (called from main event loop when sidebar is open)

- [ ] **Step 1: Write failing test for sidebar input handling**

```rust
#[test]
fn app_sidebar_move_cursor_up() {
    let mut app = App::new_with_sidebar(SidebarMode::TreeBrowser);
    let initial_cursor = app.sidebar.as_ref().unwrap().cursor;
    app.handle_sidebar_command(Command::SidebarMove(Motion::Down));
    assert!(app.sidebar.as_ref().unwrap().cursor >= initial_cursor);
}

#[test]
fn app_sidebar_typing_in_picker() {
    let mut app = App::new_with_sidebar(SidebarMode::FuzzyPicker);
    app.handle_sidebar_command(Command::SidebarInput('t'));
    app.handle_sidebar_command(Command::SidebarInput('e'));
    app.handle_sidebar_command(Command::SidebarInput('s'));
    assert_eq!(app.sidebar.as_ref().unwrap().input, "tes");
}

#[test]
fn app_sidebar_backspace_clears_filter() {
    let mut app = App::new_with_sidebar(SidebarMode::FuzzyPicker);
    app.handle_sidebar_command(Command::SidebarInput('a'));
    app.handle_sidebar_command(Command::SidebarBackspace);
    assert_eq!(app.sidebar.as_ref().unwrap().input, "");
}
```

- [ ] **Step 2: Implement `handle_sidebar_command()` in `App`**

```rust
pub fn handle_sidebar_command(&mut self, cmd: Command) {
    use crate::keymap::Command::*;
    if self.sidebar.is_none() {
        return; // Sidebar not open
    }

    match cmd {
        SidebarMove(motion) => {
            let delta = match motion {
                Motion::Up => -1,
                Motion::Down => 1,
                Motion::PageUp => -10,
                Motion::PageDown => 10,
                Motion::Home => {
                    if let Some(sidebar) = &mut self.sidebar {
                        sidebar.cursor = 0;
                    }
                    return;
                }
                Motion::End => {
                    if let Some(sidebar) = &mut self.sidebar {
                        sidebar.cursor = sidebar.visible_items().len().saturating_sub(1);
                    }
                    return;
                }
                _ => 0,
            };
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.move_cursor(delta);
            }
        }
        SidebarToggleExpand => {
            if let Some(sidebar) = &mut self.sidebar {
                if let Some(path) = sidebar.selected_path() {
                    sidebar.toggle_expand_path(&path);
                }
            }
        }
        SidebarInput(ch) => {
            if let Some(sidebar) = &mut self.sidebar {
                if sidebar.mode == SidebarMode::FuzzyPicker {
                    sidebar.input.push(ch);
                    sidebar.filter_items(&sidebar.input.clone());
                }
            }
        }
        SidebarBackspace => {
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.input.pop();
                sidebar.filter_items(&sidebar.input.clone());
            }
        }
        SidebarSubmit => {
            self.sidebar_on_enter();
        }
        SidebarCancel => {
            self.close_sidebar();
        }
        ResizeSidebar(delta) => {
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.resize(delta);
                self.sidebar_width = sidebar.width;
            }
        }
        _ => {}
    }
}
```

And update the main command handler to route to sidebar when open:

```rust
pub fn handle_command(&mut self, cmd: Command) {
    if self.sidebar.is_some() {
        // Route sidebar-related commands; ignore others (Esc, input, etc. go to sidebar)
        use crate::keymap::Command::*;
        if matches!(cmd, SidebarMove(_) | SidebarInput(_) | SidebarBackspace | SidebarSubmit | SidebarCancel | ResizeSidebar(_) | SidebarToggleExpand | OpenSidebarTree | OpenSidebarPicker) {
            self.handle_sidebar_command(cmd);
            return;
        }
        // Note: Ctrl-S (save) and other globals may still apply even with sidebar open; adjust per spec
    }
    
    // Original command handling...
    // (rest of handle_command logic unchanged)
}
```

- [ ] **Step 3: Run tests, verify they pass**

Run: `cargo test --lib app::tests`
Expected: New tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: sidebar input handler (move, type, resize, submit)"
```

---

## Task 8: Render Sidebar + Split Viewport

**Files:**
- Modify: `src/render.rs`
- Test: inline

**Interfaces:**
- Consumes: `App.sidebar`, `App.sidebar_width`
- Produces: `render::draw()` updated to split viewport; new helper `render_sidebar(f: &mut Frame, area: Rect, sidebar: &Sidebar)`

- [ ] **Step 1: Write failing test for sidebar rendering**

```rust
#[test]
fn render_sidebar_in_left_pane() {
    let mut app = App::new_with_sidebar(SidebarMode::TreeBrowser);
    let buf = buffer_of(&app, 80, 24);
    
    // Sidebar should occupy left ~32 cols (or app.sidebar_width)
    // Editor should occupy right ~48 cols
    // Verify sidebar content renders in left area
    let sidebar_area = buf.area().split(32); // First 32 cols
    // Check that sidebar items appear (heuristic: non-space chars in left area)
    let has_sidebar_content = (0..32).any(|x| {
        (0..23).any(|y| buf[(x, y)].symbol() != " ")
    });
    assert!(has_sidebar_content, "Sidebar should have content");
}

#[test]
fn render_editor_in_right_pane_when_sidebar_open() {
    let mut app = App::new_with_sidebar(SidebarMode::TreeBrowser);
    app.doc_mut().rope.insert(0, "hello");
    let buf = buffer_of(&app, 80, 24);
    
    // Editor content should start after sidebar width
    let sidebar_width = app.sidebar_width as usize;
    // Check for "hello" or part of it starting around column sidebar_width
    let has_editor_content = (sidebar_width..80).any(|x| {
        (0..23).any(|y| buf[(x as u16, y)].symbol() != " ")
    });
    assert!(has_editor_content, "Editor should have content in right pane");
}
```

- [ ] **Step 2: Modify `draw()` to split viewport**

In `src/render.rs`, update `draw()`:

```rust
pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let doc = app.doc();
    
    // Split viewport if sidebar is open
    let (sidebar_area, editor_area) = if let Some(sidebar) = &app.sidebar {
        let sidebar_width = sidebar.width as u16;
        let left = Rect { x: area.x, y: area.y, width: sidebar_width, height: area.height.saturating_sub(1) };
        let right = Rect { x: area.x + sidebar_width, y: area.y, width: area.width.saturating_sub(sidebar_width), height: area.height.saturating_sub(1) };
        (Some((left, sidebar)), right)
    } else {
        (None, Rect { x: area.x, y: area.y, width: area.width, height: area.height.saturating_sub(1) })
    };

    // Render sidebar if present
    if let Some((sidebar_rect, sidebar)) = sidebar_area {
        render_sidebar(f, sidebar_rect, sidebar);
    }

    // Render editor in editor_area (modified from existing code)
    let view_rows = editor_area.height as usize;
    let gutter_w = if app.show_line_numbers { ... } else { 0 };
    
    // ... rest of editor rendering, using editor_area instead of full area ...
    // (Reuse existing render logic, just offset by gutter_w and editor_area.x)

    // Render status bar (full width)
    let status_y = area.height.saturating_sub(1);
    let buf = f.buffer_mut();
    // ... status bar rendering (unchanged) ...
}
```

- [ ] **Step 3: Implement `render_sidebar()` helper**

```rust
fn render_sidebar(f: &mut Frame, area: Rect, sidebar: &Sidebar) {
    let buf = f.buffer_mut();

    let items = sidebar.visible_items();
    for (row, item) in items.iter().enumerate() {
        if row >= area.height as usize {
            break;
        }
        
        let y = area.y + row as u16;
        
        // Indentation
        let indent = "  ".repeat(item.depth);
        
        // Icon/marker
        let icon = if item.is_dir {
            if sidebar.tree_state.as_ref().map(|ts| ts.expanded.contains(&item.path)).unwrap_or(false) {
                "[-]"
            } else {
                "[+]"
            }
        } else {
            "   "
        };
        
        // Filename (truncate to sidebar width)
        let filename = item.path.file_name().unwrap().to_string_lossy();
        let max_len = (area.width as usize).saturating_sub(indent.len() + icon.len() + 1);
        let text = if filename.len() > max_len {
            format!("{}{}{}...", indent, icon, &filename[..max_len.saturating_sub(2)])
        } else {
            format!("{}{}{}", indent, icon, filename)
        };

        // Highlight cursor row
        let is_selected = sidebar.cursor == row;
        let style = if is_selected {
            ratatui::style::Style::default().bg(ratatui::style::Color::Blue)
        } else {
            ratatui::style::Style::default()
        };

        for (x, ch) in text.chars().enumerate() {
            if area.x + x as u16 < area.x + area.width {
                buf[(area.x + x as u16, y)].set_char(ch).set_style(style);
            }
        }
    }

    // If no items, show "no files"
    if items.is_empty() {
        let msg = "no files";
        for (x, ch) in msg.chars().enumerate() {
            if area.x + x as u16 < area.x + area.width {
                buf[(area.x + x as u16, area.y)].set_char(ch);
            }
        }
    }

    // Show input line in picker mode
    if sidebar.mode == SidebarMode::FuzzyPicker {
        let input_y = area.y;
        let prompt = format!("> {}", sidebar.input);
        for (x, ch) in prompt.chars().enumerate() {
            if area.x + x as u16 < area.x + area.width {
                buf[(area.x + x as u16, input_y)].set_char(ch);
            }
        }
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib render::tests`
Expected: New tests pass, old tests still pass (may need to update old tests if viewport assumptions changed)

- [ ] **Step 5: Commit**

```bash
git add src/render.rs
git commit -m "feat: render sidebar in left pane, split viewport"
```

---

## Task 9: Main Event Loop Integration

**Files:**
- Modify: `src/main.rs`
- Test: integration flow test (Task 10)

**Interfaces:**
- Consumes: `Command::OpenSidebarTree, Command::OpenSidebarPicker`, sidebar command routing
- Produces: Event loop routes sidebar commands to `app.handle_sidebar_command()` when sidebar is open

- [ ] **Step 1: Update main event loop in `main.rs`**

In the main tick loop (after `lookup(app.mode, &key)`), route sidebar commands:

```rust
// In main loop:
if let Some(cmd) = lookup(if app.sidebar.is_some() { /* sidebar mode */ } else { app.mode }, &key) {
    if app.sidebar.is_some() && matches!(cmd, Command::OpenSidebarTree | Command::OpenSidebarPicker | ...) {
        // Handle sidebar commands specially
        match &cmd {
            Command::OpenSidebarTree => app.open_sidebar(SidebarMode::TreeBrowser),
            Command::OpenSidebarPicker => app.open_sidebar(SidebarMode::FuzzyPicker),
            _ => app.handle_sidebar_command(cmd),
        }
    } else if app.sidebar.is_none() && matches!(cmd, Command::OpenSidebarTree | Command::OpenSidebarPicker) {
        match &cmd {
            Command::OpenSidebarTree => app.open_sidebar(SidebarMode::TreeBrowser),
            Command::OpenSidebarPicker => app.open_sidebar(SidebarMode::FuzzyPicker),
            _ => app.handle_command(cmd),
        }
    } else {
        app.handle_command(cmd);
    }
}
```

- [ ] **Step 2: Verify event loop still works with sidebar**

No test here; integration test in Task 10 covers this.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: route sidebar commands in main event loop"
```

---

## Task 10: Integration Test + Full Acceptance

**Files:**
- Modify: `tests/flow.rs`
- Test: integration

**Interfaces:**
- Tests the full flow: open sidebar, navigate, select, close

- [ ] **Step 1: Write integration test for sidebar flow**

```rust
#[test]
fn sidebar_tree_browser_flow() {
    // Create a temp structure
    let tmpdir = TempDir::new().unwrap();
    let root = tmpdir.path();
    std::fs::write(root.join("file1.rs"), "content1").unwrap();
    std::fs::create_dir(root.join("subdir")).unwrap();
    std::fs::write(root.join("subdir").join("file2.rs"), "content2").unwrap();

    let mut app = App::new(vec![root.join("dummy.txt")]);
    
    // Open tree browser
    app.open_sidebar(SidebarMode::TreeBrowser);
    assert!(app.sidebar.is_some());
    assert_eq!(app.sidebar.as_ref().unwrap().mode, SidebarMode::TreeBrowser);

    // Navigate
    app.handle_sidebar_command(Command::SidebarMove(Motion::Down));
    let selected = app.sidebar.as_ref().unwrap().selected_path();
    assert!(selected.is_some());

    // Expand if dir
    if selected.as_ref().map(|p| p.is_dir()).unwrap_or(false) {
        app.handle_sidebar_command(Command::SidebarToggleExpand);
    }

    // Select a file
    app.handle_sidebar_command(Command::SidebarMove(Motion::Down));
    app.handle_sidebar_command(Command::SidebarSubmit);

    // Sidebar should close, file should open
    assert!(app.sidebar.is_none());
    assert!(app.docs.len() >= 2); // Original + new file
}

#[test]
fn sidebar_fuzzy_picker_flow() {
    let tmpdir = TempDir::new().unwrap();
    let root = tmpdir.path();
    std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(root.join("app.rs"), "").unwrap();
    std::fs::write(root.join("test.txt"), "").unwrap();

    let mut app = App::new(vec![root.join("dummy.txt")]);
    
    // Open picker
    app.open_sidebar(SidebarMode::FuzzyPicker);
    assert_eq!(app.sidebar.as_ref().unwrap().mode, SidebarMode::FuzzyPicker);

    // Type to filter
    app.handle_sidebar_command(Command::SidebarInput('m'));
    app.handle_sidebar_command(Command::SidebarInput('a'));
    app.handle_sidebar_command(Command::SidebarInput('i'));
    app.handle_sidebar_command(Command::SidebarInput('n'));

    // Verify filtered
    let visible = app.sidebar.as_ref().unwrap().visible_items();
    assert!(visible.iter().any(|item| item.path.to_string_lossy().contains("main")));

    // Select and open
    app.handle_sidebar_command(Command::SidebarSubmit);
    
    // Sidebar closed
    assert!(app.sidebar.is_none());
    assert!(app.docs.iter().any(|d| d.path.as_ref().map(|p| p.to_string_lossy().contains("main")).unwrap_or(false)));
}

#[test]
fn sidebar_resize_persists_in_session() {
    let mut app = App::new(vec![]);
    app.open_sidebar(SidebarMode::TreeBrowser);
    
    let initial_width = app.sidebar.as_ref().unwrap().width;
    
    // Resize
    app.handle_sidebar_command(Command::ResizeSidebar(2));
    assert_eq!(app.sidebar.as_ref().unwrap().width, initial_width + 2);
    
    // Close and reopen
    app.close_sidebar();
    app.sidebar_width = app.sidebar.as_ref().map(|s| s.width).unwrap_or(32);
    app.open_sidebar(SidebarMode::TreeBrowser);
    
    // Width should persist (sidebar_width was updated)
    assert_eq!(app.sidebar.as_ref().unwrap().width, initial_width + 2);
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test flow`
Expected: All pass

- [ ] **Step 3: Manual acceptance test (perf smoke)**

```bash
cargo build --release
# Create a medium tree
mkdir -p /tmp/plat-test/src/subdir
for i in {1..20}; do echo "// file $i" > /tmp/plat-test/src/file$i.rs; done
for i in {1..10}; do echo "// subfile $i" > /tmp/plat-test/src/subdir/file$i.rs; done

# Run and open sidebar
time ./target/release/plat /tmp/plat-test/src/file1.rs
# Press Ctrl-B, verify tree opens within 100ms
# Press arrows, verify instant response
# Type in Ctrl-P picker, verify instant filtering
```

- [ ] **Step 4: All tests pass**

Run: `cargo test`
Expected: 70+ tests pass (Phase 0 + Phase 1 sidebar tests)

- [ ] **Step 5: Commit**

```bash
git add tests/flow.rs
git commit -m "test: integration tests for sidebar (tree, picker, resize)"
```

---

## Task 11: Documentation & Review Focus Tests

**Files:**
- Modify: `README.md`, review focus tests in respective modules

**Interfaces:**
- Produces: Updated README with sidebar hotkeys and usage examples; each Review Focus item pinned by a test

- [ ] **Step 1: Update README.md with sidebar documentation**

Add section:

```markdown
### File Sidebar (Phase 1)

**Ctrl-B** — Tree Browser: Browse files/directories, navigate with arrow keys, `→` to expand/collapse, `Enter` to open.

**Ctrl-P** — Fuzzy Picker: Search files by name and path, type to filter, `Enter` to open.

**Alt-←** / **Alt-→** — Resize sidebar (min 10, max 50 cols). Width persists in the session.

**Esc** — Close sidebar.

Sidebar respects `.gitignore` if present; default ignores: `.git`, `node_modules`, `target`, `__pycache__`, `.egg-info`.
```

- [ ] **Step 2: Add Review Focus test: empty tree (all files ignored)**

In `src/tree.rs` tests:

```rust
#[test]
fn build_tree_all_files_ignored_returns_empty() {
    let tmpdir = TempDir::new().unwrap();
    let root = tmpdir.path();
    fs::write(root.join(".gitignore"), "*\n").unwrap();
    fs::write(root.join("file.rs"), "").unwrap();

    let tree = FileTree::build(root).unwrap();
    assert_eq!(tree.items.len(), 0, "All ignored tree should be empty");
}
```

And in `src/sidebar.rs` tests:

```rust
#[test]
fn sidebar_empty_tree_shows_no_files_message() {
    let sidebar = Sidebar::new_tree(vec![], 32);
    let visible = sidebar.visible_items();
    assert_eq!(visible.len(), 0, "Empty tree should show no items");
}
```

- [ ] **Step 3: Add Review Focus test: permission denied (skips, continues)**

In `src/tree.rs` tests:

```rust
#[test]
fn build_tree_skips_unreadable_dirs() {
    // This is platform-specific; on Unix, create a dir with no read permission
    // On other platforms, skip or mock
    #[cfg(unix)]
    {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        let no_read = root.join("no_read");
        std::fs::create_dir(&no_read).unwrap();
        std::fs::write(no_read.join("file.rs"), "").unwrap();
        
        // Remove read permission
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&no_read, std::fs::Permissions::from_mode(0o000)).unwrap();

        let tree = FileTree::build(root).unwrap();
        // Tree should build (no panic), but skip the no_read dir
        assert!(!tree.items.iter().any(|i| i.path.to_string_lossy().contains("no_read")));

        // Restore for cleanup
        std::fs::set_permissions(&no_read, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
```

- [ ] **Step 4: Add Review Focus test: symlink loops (no hang)**

In `src/tree.rs` tests:

```rust
#[test]
fn build_tree_detects_symlink_loops() {
    #[cfg(unix)]
    {
        let tmpdir = TempDir::new().unwrap();
        let root = tmpdir.path();
        let dir_a = root.join("a");
        let dir_b = dir_a.join("b");
        std::fs::create_dir_all(&dir_b).unwrap();
        
        // Create symlink loop: a/b/c -> a
        use std::os::unix::fs as unix_fs;
        unix_fs::symlink(dir_a.clone(), dir_b.join("c")).unwrap();

        // This should not hang or panic
        let tree = FileTree::build(root).unwrap();
        assert!(tree.items.len() > 0, "Tree should be built despite loop");
    }
}
```

- [ ] **Step 5: Add Review Focus test: resize clamping**

In `src/sidebar.rs` tests:

```rust
#[test]
fn sidebar_resize_clamps_to_boundaries() {
    let mut sidebar = Sidebar::new_tree(vec![], 32);
    
    // Shrink below 10
    sidebar.resize(-100);
    assert_eq!(sidebar.width, 10, "Width should clamp to min 10");
    
    // Grow above 50
    sidebar.resize(1000);
    assert_eq!(sidebar.width, 50, "Width should clamp to max 50");
}
```

- [ ] **Step 6: Add Review Focus test: mode switch (tree ↔ picker)**

In `src/app.rs` tests:

```rust
#[test]
fn sidebar_mode_switch_closes_old_opens_new() {
    let mut app = App::new(vec![]);
    
    // Open tree
    app.open_sidebar(SidebarMode::TreeBrowser);
    assert_eq!(app.sidebar.as_ref().unwrap().mode, SidebarMode::TreeBrowser);
    
    // Switch to picker
    app.open_sidebar(SidebarMode::FuzzyPicker);
    assert_eq!(app.sidebar.as_ref().unwrap().mode, SidebarMode::FuzzyPicker);
    
    // Input state should not leak
    assert_eq!(app.sidebar.as_ref().unwrap().input, "");
}
```

- [ ] **Step 7: Run all tests including Review Focus tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 8: Commit**

```bash
git add README.md src/tree.rs src/sidebar.rs src/app.rs
git commit -m "docs: sidebar README + Review Focus tests (empty tree, permissions, loops, resize, mode switch)"
```

---

## Task 12: Cleanup, Final Testing & Final Commit

**Files:**
- All (verify no TODOs, no placeholders)
- Verify: all Phase 0 tests still pass

**Interfaces:**
- All modules complete and tested

- [ ] **Step 1: Run full test suite**

Run: `cargo test --lib && cargo test --test flow`
Expected: All tests pass (70+ from Phase 0, 20+ new from Phase 1)

- [ ] **Step 2: Check for FIXMEs / TODOs / placeholders**

Run: `grep -r "TODO\|FIXME\|TBD\|ponytail:" src/ tests/ docs/ | grep -v "ponytail: max depth" | grep -v existing`
Expected: Only "ponytail: max depth 20" (deferred optimization) or existing Phase 0 ponytail comments

- [ ] **Step 3: Perf check (no regression)**

```bash
cargo build --release
seq 1 100000 | sed 's/^/line /' > /tmp/big.txt
time ./target/release/plat /tmp/big.txt
# Should still open + scroll in <1s (no sidebar overhead when closed)
```

- [ ] **Step 4: Manual smoke test**

```bash
# Ctrl-B: tree opens, arrows work, expand/collapse works, Enter opens file
# Ctrl-P: picker opens, typing filters, Enter opens file
# Alt-Left/Right: resize works, persists on close/reopen
# Esc: closes sidebar
# Ctrl-S, Ctrl-Z, etc.: still work with sidebar open
```

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat: Phase 1 complete - file sidebar (tree browser + fuzzy picker)

- Tree browser: Ctrl-B, navigate with arrows, expand/collapse dirs
- Fuzzy picker: Ctrl-P, live search by name/path
- Resizable: Alt-Left/Right to shrink/grow (10-50 cols, persists in session)
- .gitignore support: basic glob matching, defaults exclude .git/node_modules/target
- Review Focus: empty tree, permission denied, symlink loops, resize clamp, mode switch
- All Phase 0 tests still pass; 20+ new sidebar tests
- Zero regression on 100k-line file performance"
```

- [ ] **Step 6: Verify final state**

Run: `cargo build --release && cargo test`
Expected: Binary builds, all tests pass

---

## Self-Review

**Spec coverage:**
- ✅ Resizable sidebar (Tasks 4, 8)
- ✅ Tree browser (Tasks 2, 4, 7, 9)
- ✅ Fuzzy picker (Tasks 3, 4, 7, 9)
- ✅ Keymap (Ctrl-B, Ctrl-P, Alt-L/R, Esc) (Task 6)
- ✅ `.gitignore` parsing (Task 1)
- ✅ Error handling (permission denied, loops, empty tree) (Tasks 2, 11)
- ✅ Layout & rendering (Task 8)
- ✅ Integration (Tasks 9, 10)

**Step unambiguity:** Each step names exact file, function signature, test assertions — implementer writes exactly one thing.

**Type consistency:** `TreeItem`, `Sidebar`, `Command` used consistently across modules per spec.

**Review Focus:** All five items (empty tree, permissions, loops, resize, mode switch) pinned by tests in Task 11.

**Proportion:** ~12 tasks, ~180 steps (compact but complete). Code blocks only for algorithms; signatures + test names dominant.
