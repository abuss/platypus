use crate::picker::FuzzyMatcher;
use crate::tree::TreeItem;
use std::collections::HashSet;
use std::path::PathBuf;

pub const MIN_WIDTH: u16 = 10;
pub const MAX_WIDTH: u16 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    filtered: Vec<TreeItem>,
}

fn clamp_width(width: u16) -> u16 {
    width.max(MIN_WIDTH).min(MAX_WIDTH)
}

impl Sidebar {
    pub fn new_tree(items: Vec<TreeItem>, width: u16) -> Self {
        Sidebar {
            mode: SidebarMode::TreeBrowser,
            width: clamp_width(width),
            cursor: 0,
            input: String::new(),
            tree_state: Some(TreeState { items, expanded: HashSet::new() }),
            matcher: None,
            filtered: Vec::new(),
        }
    }

    pub fn new_picker(items: Vec<TreeItem>, width: u16) -> Self {
        let matcher = FuzzyMatcher::new(items);
        let filtered = matcher.search("").into_iter().map(|r| r.item).collect();
        Sidebar {
            mode: SidebarMode::FuzzyPicker,
            width: clamp_width(width),
            cursor: 0,
            input: String::new(),
            tree_state: None,
            matcher: Some(matcher),
            filtered,
        }
    }

    /// Rows actually on screen: a tree item is hidden when its parent dir is
    /// not expanded; picker rows are already filtered, so they all show.
    pub fn visible_items(&self) -> Vec<&TreeItem> {
        match self.mode {
            SidebarMode::FuzzyPicker => self.filtered.iter().collect(),
            SidebarMode::TreeBrowser => match &self.tree_state {
                Some(ts) => ts
                    .items
                    .iter()
                    .filter(|item| {
                        item.depth == 0
                            || item
                                .path
                                .parent()
                                .is_some_and(|p| ts.expanded.contains(&p.to_path_buf()))
                    })
                    .collect(),
                None => Vec::new(),
            },
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let count = self.visible_items().len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        let pos = self.cursor as i32 + delta;
        self.cursor = pos.clamp(0, count as i32 - 1) as usize;
    }

    pub fn toggle_expand_path(&mut self, path: &PathBuf) {
        if let Some(ts) = &mut self.tree_state {
            if !ts.expanded.insert(path.clone()) {
                ts.expanded.remove(path);
            }
            self.cursor = self
                .cursor
                .min(self.visible_items().len().saturating_sub(1));
        }
    }

    pub fn expand_path(&mut self, path: &PathBuf) {
        if let Some(ts) = &mut self.tree_state {
            ts.expanded.insert(path.clone());
            self.cursor = self
                .cursor
                .min(self.visible_items().len().saturating_sub(1));
        }
    }

    pub fn collapse_or_parent(&mut self, path: &PathBuf) {
        if let Some(ts) = &mut self.tree_state {
            if ts.expanded.contains(path) {
                ts.expanded.remove(path);
                self.cursor = self
                    .cursor
                    .min(self.visible_items().len().saturating_sub(1));
            } else if let Some(parent) = path.parent() {
                let parent_path = parent.to_path_buf();
                if let Some(pos) = self.visible_items().iter().position(|i| i.path == parent_path) {
                    self.cursor = pos;
                }
            }
        }
    }

    pub fn filter_items(&mut self, term: &str) {
        self.input = term.to_string();
        if let Some(matcher) = &self.matcher {
            self.filtered = matcher.search(term).into_iter().map(|r| r.item).collect();
            self.cursor = 0;
        }
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.visible_items().get(self.cursor).map(|i| i.path.clone())
    }

    pub fn resize(&mut self, delta: i16) {
        self.width = clamp_width((self.width as i16 + delta).max(0) as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeItem;
    use std::path::PathBuf;

    fn items() -> Vec<TreeItem> {
        vec![
            TreeItem { path: PathBuf::from("file1.rs"), is_dir: false, depth: 0 },
            TreeItem { path: PathBuf::from("dir"), is_dir: true, depth: 0 },
            TreeItem { path: PathBuf::from("dir/file2.rs"), is_dir: false, depth: 1 },
        ]
    }

    fn sb_item(path: &str, is_dir: bool, depth: usize) -> TreeItem {
        TreeItem { path: PathBuf::from(path), is_dir, depth }
    }

    #[test]
    fn sidebar_move_cursor_up_down() {
        let mut sidebar = Sidebar::new_tree(items(), 32);
        assert_eq!(sidebar.cursor, 0);
        sidebar.move_cursor(1);
        assert_eq!(sidebar.cursor, 1);
        sidebar.move_cursor(-1);
        assert_eq!(sidebar.cursor, 0);
    }

    #[test]
    fn sidebar_cursor_clamps_to_bounds() {
        let mut sidebar = Sidebar::new_tree(items(), 32);
        sidebar.move_cursor(100);
        assert_eq!(sidebar.cursor, sidebar.visible_items().len() - 1);
        sidebar.move_cursor(-100);
        assert_eq!(sidebar.cursor, 0);
    }

    #[test]
    fn sidebar_toggle_expand_adds_to_expanded_set() {
        let mut sidebar = Sidebar::new_tree(items(), 32);
        let dir = PathBuf::from("dir");
        sidebar.toggle_expand_path(&dir);
        assert!(sidebar.tree_state.as_ref().unwrap().expanded.contains(&dir));
        assert!(sidebar.visible_items().iter().any(|i| i.path == PathBuf::from("dir/file2.rs")));
        sidebar.toggle_expand_path(&dir);
        assert!(sidebar.tree_state.as_ref().unwrap().expanded.is_empty());
        assert!(!sidebar.visible_items().iter().any(|i| i.path == PathBuf::from("dir/file2.rs")));
    }

    #[test]
    fn sidebar_selected_path_returns_current_item() {
        let mut sidebar = Sidebar::new_tree(items(), 32);
        sidebar.cursor = 1;
        assert_eq!(sidebar.selected_path(), Some(PathBuf::from("dir")));
    }

    #[test]
    fn sidebar_picker_filter_updates_visible_items() {
        let mut sidebar = Sidebar::new_picker(items(), 32);
        sidebar.filter_items("file1");
        let visible = sidebar.visible_items();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, PathBuf::from("file1.rs"));
    }

    #[test]
    fn empty_tree_has_no_rows_and_survives_navigation() {
        // Review Focus: all files ignored -> nothing to pick, Esc path stays clean
        let mut sidebar = Sidebar::new_tree(vec![], 32);
        assert_eq!(sidebar.visible_items().len(), 0);
        sidebar.move_cursor(-5);
        sidebar.move_cursor(5);
        assert_eq!(sidebar.cursor, 0);
        assert_eq!(sidebar.selected_path(), None);
    }

    #[test]
    fn resize_clamps_to_the_spec_bounds() {
        // Review Focus: 10..50 cols, clamped not wrapped
        let mut sidebar = Sidebar::new_tree(vec![], 32);
        sidebar.resize(-1000);
        assert_eq!(sidebar.width, MIN_WIDTH);
        sidebar.resize(1000);
        assert_eq!(sidebar.width, MAX_WIDTH);
        assert_eq!(Sidebar::new_tree(vec![], 5).width, MIN_WIDTH, "constructor clamps too");
        assert_eq!(Sidebar::new_tree(vec![], 9999).width, MAX_WIDTH);
    }

    #[test]
    fn left_arrow_collapses_then_walks_to_parent() {
        let items = vec![sb_item("src", true, 0), sb_item("src/app.rs", false, 1)];
        let mut sb = Sidebar::new_tree(items, 30);
        sb.cursor = 1;                       // on src/app.rs
        let p = PathBuf::from("src");
        sb.expand_path(&p);                  // → expands
        assert!(sb.tree_state.as_ref().unwrap().expanded.contains(&p));
        sb.collapse_or_parent(&PathBuf::from("src/app.rs")); // ← on a file: jump to parent row "src"
        assert_eq!(sb.selected_path(), Some(PathBuf::from("src")));
        sb.collapse_or_parent(&p);           // ← on an expanded dir: collapse it
        assert!(!sb.tree_state.as_ref().unwrap().expanded.contains(&p));
        assert_eq!(sb.selected_path(), Some(p.clone())); // cursor stays on collapsed dir's row
    }
}
