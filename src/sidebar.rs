use crate::picker::{subsequence_match, subsequence_score, FuzzyMatcher};
use crate::tree::TreeItem;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    /// Tree mode: Some(...) while a filter is active (ancestor-kept subtree).
    tree_filtered: Option<Vec<TreeItem>>,
    /// Matched byte offsets per row of `filtered`/`tree_filtered` (for
    /// underlines); empty vec per row when the row has no name-level hit.
    ranges: Vec<Vec<usize>>,
    /// Virtual ".." row that leads an absolute-path tree: Enter on it makes
    /// the parent the new root.
    root_dot_item: Option<TreeItem>,
}

fn name_of(item: &TreeItem) -> String {
    item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn clamp_width(width: u16) -> u16 {
    width.max(MIN_WIDTH).min(MAX_WIDTH)
}

impl Sidebar {
    pub fn new_tree(items: Vec<TreeItem>, width: u16) -> Self {
        // Production trees carry absolute paths; the ".." row targets the
        // parent of the tree root (= parent of any depth-0 item's parent).
        let root_dot_item = items
            .iter()
            .find(|i| i.depth == 0)
            .and_then(|i| i.path.parent())
            .and_then(|root| root.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| TreeItem { path: p.to_path_buf(), is_dir: true, depth: 0 });
        Sidebar {
            mode: SidebarMode::TreeBrowser,
            width: clamp_width(width),
            cursor: 0,
            input: String::new(),
            tree_state: Some(TreeState { items, expanded: HashSet::new() }),
            matcher: None,
            filtered: Vec::new(),
            tree_filtered: None,
            ranges: Vec::new(),
            root_dot_item,
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
            tree_filtered: None,
            ranges: Vec::new(),
            root_dot_item: None,
        }
    }

    /// The virtual ".." path, when the tree has one (absolute-path roots).
    pub fn root_dot(&self) -> Option<&Path> {
        self.root_dot_item.as_ref().map(|i| i.path.as_path())
    }

    /// Point the ".." row at `root`'s parent (the app knows the real scan
    /// root; this also covers trees with zero items).
    pub fn set_tree_root(&mut self, root: &Path) {
        self.root_dot_item = root
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| TreeItem { path: p.to_path_buf(), is_dir: true, depth: 0 });
    }

    /// Rows actually on screen: ".." (tree only, always), then either the
    /// filtered view (picker, or tree with non-empty input) or the
    /// expand/collapse-based tree view.
    pub fn visible_items(&self) -> Vec<&TreeItem> {
        let mut out: Vec<&TreeItem> = Vec::new();
        if let Some(rd) = &self.root_dot_item {
            out.push(rd);
        }
        match self.mode {
            SidebarMode::FuzzyPicker => out.extend(self.filtered.iter()),
            SidebarMode::TreeBrowser => match (&self.tree_filtered, &self.tree_state) {
                (Some(tf), _) => out.extend(tf.iter()),
                (None, Some(ts)) => out.extend(ts.items.iter().filter(|item| {
                    item.depth == 0
                        || item
                            .path
                            .parent()
                            .is_some_and(|p| ts.expanded.contains(&p.to_path_buf()))
                })),
                (None, None) => {}
            },
        }
        out
    }

    /// Byte offsets of filter hits per visible row (aligned with
    /// `visible_items()`; empty vec for rows without a name-level hit).
    pub fn match_ranges(&self) -> Vec<Vec<usize>> {
        let vis_n = self.visible_items().len();
        let filtered_active = match self.mode {
            SidebarMode::FuzzyPicker => true,
            SidebarMode::TreeBrowser => self.tree_filtered.is_some(),
        };
        if !filtered_active {
            return vec![Vec::new(); vis_n];
        }
        let mut out = Vec::with_capacity(vis_n);
        if self.root_dot_item.is_some() {
            out.push(Vec::new());
        }
        out.extend(self.ranges.iter().cloned());
        out.truncate(vis_n);
        out
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

    /// Re-run the filter for both modes: picker keeps score order, the tree
    /// keeps structural (alphabetical) order and the ancestors of matches.
    pub fn filter_items(&mut self, term: &str) {
        self.input = term.to_string();
        self.ranges.clear();
        match self.mode {
            SidebarMode::FuzzyPicker => {
                let results = self.matcher.as_ref().map(|m| m.search(term)).unwrap_or_default();
                self.filtered = results.iter().map(|r| r.item.clone()).collect();
                self.ranges = results
                    .iter()
                    .map(|r| {
                        let name = name_of(&r.item);
                        subsequence_match(&name, term).map(|(_, hits)| hits).unwrap_or_default()
                    })
                    .collect();
                self.cursor = 0;
            }
            SidebarMode::TreeBrowser => {
                if term.is_empty() {
                    self.tree_filtered = None;
                    self.cursor = self.cursor.min(self.visible_items().len().saturating_sub(1));
                    return;
                }
                let Some(ts) = &self.tree_state else { return };
                let by_path: HashMap<&Path, usize> =
                    ts.items.iter().enumerate().map(|(i, it)| (it.path.as_path(), i)).collect();
                let mut keep = HashSet::<usize>::new();
                for (idx, it) in ts.items.iter().enumerate() {
                    let name = name_of(it);
                    let rel = it.path.to_string_lossy().into_owned();
                    if subsequence_match(&name, term).is_some()
                        || subsequence_score(&rel, term).is_some()
                    {
                        keep.insert(idx);
                        let mut anc = it.path.parent();
                        while let Some(a) = anc {
                            match by_path.get(a) {
                                Some(&ai) => {
                                    keep.insert(ai);
                                    anc = a.parent();
                                }
                                None => break,
                            }
                        }
                    }
                }
                let mut rows = Vec::new();
                for (idx, it) in ts.items.iter().enumerate() {
                    if keep.contains(&idx) {
                        rows.push(it.clone());
                        let name = name_of(it);
                        self.ranges.push(
                            subsequence_match(&name, term).map(|(_, hits)| hits).unwrap_or_default(),
                        );
                    }
                }
                self.tree_filtered = Some(rows);
                self.cursor = 0;
            }
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

    #[test]
    fn tree_filter_keeps_matches_and_ancestors_in_tree_order() {
        let mut sb = Sidebar::new_tree(vec![
            sb_item("docs", true, 0), sb_item("docs/read.md", false, 1),
            sb_item("src", true, 0), sb_item("src/main.rs", false, 1),
        ], 30);
        sb.filter_items("main");
        let vis: Vec<String> = sb.visible_items().iter()
            .map(|i| i.path.to_string_lossy().into_owned()).collect();
        assert_eq!(vis, vec!["src", "src/main.rs"], "ancestor kept, docs pruned: {vis:?}");
        sb.filter_items("");
        let vis2: Vec<String> = sb.visible_items().iter()
            .map(|i| i.path.to_string_lossy().into_owned()).collect();
        assert_eq!(vis2, vec!["docs", "src"], "clearing returns the collapsed view: {vis2:?}");
    }

    #[test]
    fn filter_match_ranges_point_into_the_name() {
        let mut sb = Sidebar::new_tree(vec![
            sb_item("src", true, 0), sb_item("src/main.rs", false, 1),
        ], 30);
        sb.filter_items("ain");
        let ranges = sb.match_ranges().to_vec();
        assert_eq!(ranges.len(), sb.visible_items().len(), "ranges align with visible rows");
        assert!(ranges.iter().next().map(|r| r.is_empty()).unwrap_or(false), "ancestor row not underlined");
        let idx = sb.visible_items().iter().position(|i| i.path == PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(ranges[idx], vec![1, 2, 3], "\"ain\" at bytes 1..=3 of main.rs");
    }

    #[test]
    fn picker_rows_get_match_ranges_too() {
        let mut sb = Sidebar::new_picker(vec![sb_item("src/main.rs", false, 1)], 30);
        sb.filter_items("ain");
        let ranges = sb.match_ranges().to_vec();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], vec![1, 2, 3]);
    }

    #[test]
    fn root_dot_row_leads_the_tree_when_paths_are_absolute() {
        // production trees carry absolute paths; the ".." row must be the
        // first visible row so Enter on it re-roots one level up.
        let root = std::env::temp_dir().join(format!("plat-rootdot-{}", std::process::id()));
        let items = vec![TreeItem { path: root.join("a.rs"), is_dir: false, depth: 0 }];
        let sb = Sidebar::new_tree(items, 30);
        assert_eq!(sb.root_dot(), Some(root.parent().unwrap()));
        assert_eq!(sb.visible_items()[0].path, root.parent().unwrap(), ".. row must be first");
        assert_eq!(sb.match_ranges()[0], Vec::<usize>::new(), "the .. row never underlines");
        // relative-path unit trees (the other tests) show no .. row
        let plain = Sidebar::new_tree(vec![sb_item("a.rs", false, 0)], 30);
        assert_eq!(plain.root_dot(), None);
        assert_eq!(plain.visible_items().len(), 1);
    }
}
