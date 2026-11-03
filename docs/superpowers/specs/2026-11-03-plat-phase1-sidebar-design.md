# plat Phase 1 — File Sidebar Design

Date: 2026-11-03  
Status: approved by user  
Parent doc: PLAN.md (Phase 1 section)

## Purpose

Add a resizable left sidebar with two file-navigation modes to plat:
- **Tree Browser** (`Ctrl-B`): Hierarchical file tree, navigate with arrows, expand/collapse directories
- **Fuzzy Picker** (`Ctrl-P`): Live-filtering search by filename and full path

Both modes replace the current `Ctrl-P` open prompt. Sidebar is resizable with arrow keys, closes after opening a file, and respects `.gitignore` when present.

---

## Overview

### User Flow

**Tree Browser:**
1. Press `Ctrl-B` → sidebar opens on left, shows directory tree (all dirs collapsed initially)
2. Arrows: navigate up/down
3. Right arrow: expand directory (or no-op if file)
4. Left arrow: collapse directory or jump to parent
5. Enter: open file and close sidebar, or expand directory
6. Esc: close sidebar, return focus to editor

**Fuzzy Picker:**
1. Press `Ctrl-P` → sidebar opens, shows all files (flat list initially)
2. Type to filter (live search across relative paths and filenames)
3. Arrows: navigate filtered results
4. Enter: open selected file and close sidebar
5. Esc: close sidebar without opening

**Resize:**
- `Alt-←` / `Alt-→`: shrink/grow sidebar (min 10 cols, max 50 cols)
- Width persists across toggles in the session (resets on restart)

### Layout

```
┌────────────┬──────────────────┐
│ Sidebar    │  Editor Buffer   │
│ (resizable)│ (cursor here by  │
│            │  default)        │
├────────────┴──────────────────┤
│ Status bar (full width)        │
└────────────────────────────────┘
```

Sidebar width is adjustable; minimum 10 chars (just filenames truncated), maximum 50 chars.

---

## Architecture

### New Modules

#### `sidebar.rs` — Sidebar state and controller
```rust
pub enum SidebarMode {
    TreeBrowser,
    FuzzyPicker,
}

pub struct Sidebar {
    pub mode: SidebarMode,
    pub width: u16,  // 10..=50, default 32
    pub cursor: usize,  // selected item index in current view
    pub input: String,  // search/filter text (picker only)
    pub tree: TreeState,  // current tree view state (browser only)
}

pub struct TreeState {
    pub items: Vec<TreeItem>,  // flat list of visible items + their depth
    pub expanded: HashSet<PathBuf>,  // dirs currently expanded
}

pub struct TreeItem {
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,  // indentation level
    pub visible: bool,  // passes current filter (picker) or is_ancestor_expanded (browser)
}

impl Sidebar {
    pub fn new(mode: SidebarMode, width: u16, root: &Path) -> Self;
    pub fn move_cursor(&mut self, delta: i32);  // up/down navigation
    pub fn toggle_expand(&mut self, idx: usize);  // browser: expand/collapse dir
    pub fn filter_items(&mut self, term: &str);  // picker: live search
    pub fn selected_path(&self) -> Option<PathBuf>;  // currently selected item
}
```

#### `tree.rs` — Directory tree building and traversal
```rust
pub struct FileTree {
    pub root: PathBuf,
    pub items: Vec<TreeItem>,  // sorted: dirs first, then files, alphabetically
    pub gitignore: Option<GitignoreRules>,  // if .gitignore exists
}

pub struct GitignoreRules {
    pub rules: Vec<String>,  // parsed patterns (Phase 1: glob-simple, not full gitignore spec)
}

impl FileTree {
    pub fn build(root: &Path) -> Result<Self, String>;  // scan dir, respect .gitignore
    pub fn is_ignored(&self, path: &Path) -> bool;
    pub fn children_of(&self, parent: &Path) -> Vec<TreeItem>;  // dirs only, for expand/collapse
}
```

#### `picker.rs` — Fuzzy search engine
```rust
pub struct FuzzyMatcher {
    pub all_items: Vec<TreeItem>,  // all files in tree
}

pub struct FuzzyResult {
    pub item: TreeItem,
    pub score: u32,  // higher = better match
    pub match_pos: Vec<usize>,  // character positions of matches (for highlighting, Phase 1.1)
}

impl FuzzyMatcher {
    pub fn new(tree: &FileTree) -> Self;
    pub fn search(&self, term: &str) -> Vec<FuzzyResult>;  // live filter on basename + path
}
```

### Modified Modules

#### `app.rs`
Add:
```rust
pub struct App {
    // ... existing fields ...
    pub sidebar: Option<Sidebar>,  // None when closed
    pub sidebar_width: u16,  // persists across open/close in session
}

impl App {
    pub fn open_sidebar(&mut self, mode: SidebarMode);
    pub fn close_sidebar(&mut self);
    pub fn sidebar_selected(&mut self);  // open the selected file
    pub fn handle_sidebar_input(&mut self, cmd: Command);  // motion/expand/filter/submit
}
```

#### `render.rs`
Modify `draw()` to:
1. Calculate split: `sidebar_width` on left, remainder for editor
2. Render sidebar in its area (tree items or picker results with live highlights)
3. Render editor buffer in its area (same as before, just narrower)
4. Render status bar (full width, indicates sidebar mode if open)

Add helper:
```rust
pub fn render_sidebar(f: &mut Frame, area: Rect, sidebar: &Sidebar);
pub fn apply_sidebar_resize(app: &mut App, delta: i16);  // shrink/grow width
```

#### `keymap.rs`
Change:
- Remove `Open` command (old Ctrl-P prompt)
- Add `OpenSidebarTree` (Ctrl-B)
- Add `OpenSidebarPicker` (Ctrl-P)
- Add `SidebarMove(Motion)` (Up/Down to move cursor)
- Add `SidebarToggleExpand` (Right/Left in tree to expand/collapse)
- Add `SidebarInput(char)` (typing in picker filter)
- Add `SidebarBackspace` (clear filter char)
- Add `SidebarSubmit` (Enter to open file)
- Add `SidebarCancel` (Esc to close)
- Add `ResizeSidebar(i16)` (Alt-Left/Right to shrink/grow)

Update `lookup()` to route to sidebar commands when `App.sidebar.is_some()`.

#### `input.rs`
Add:
- Crossterm Alt-Left/Right key decoding

#### `main.rs`
No changes (sidebar lifecycle managed by app state).

---

## Data Flow

### Startup
1. `App::new()` initializes with `sidebar: None`
2. On first Ctrl-B or Ctrl-P: `App::open_sidebar(mode)` → calls `FileTree::build(cwd)` → `Sidebar::new()` → renders tree or picker

### Tree Building (`FileTree::build`)
```
1. Scan root directory recursively (DFS or BFS)
2. For each entry:
   a. Check .gitignore rules if present (default ignores: .git, node_modules, target)
   b. If ignored, skip
   c. If dir, add to items with is_dir=true
   d. If file, add to items with is_dir=false
3. Sort: dirs first (alphabetically), then files (alphabetically)
4. Return FileTree { items, gitignore_rules }
```

### Sidebar Navigation (Tree Browser)
```
User presses Right Arrow:
1. lookup() → SidebarToggleExpand
2. handle_sidebar_input(SidebarToggleExpand)
3. Sidebar.toggle_expand(cursor) → add item.path to expanded set
4. Rebuild visible items (TreeState.items) from FileTree, respecting expanded set
5. Render with indentation per depth
```

### Sidebar Navigation (Fuzzy Picker)
```
User types "app":
1. Each keystroke: lookup() → SidebarInput('a'), SidebarInput('p'), SidebarInput('p')
2. handle_sidebar_input(SidebarInput(ch)) → sidebar.input.push(ch)
3. sidebar.filter_items(input) calls FuzzyMatcher.search()
4. Rebuild sidebar.items with matching files only
5. Render filtered list
```

### Opening a File
```
User presses Enter on a file:
1. lookup() → SidebarSubmit
2. handle_sidebar_input(SidebarSubmit)
3. sidebar.selected_path() → Some(path)
4. app.open_path(path) (reuse from Phase 0)
5. sidebar closes (app.sidebar = None)
6. New buffer becomes active
```

---

## Detailed Behavior

### Tree Browser

**Initial state:** All directories collapsed, cursor on first item (usually `.` or first file)

**Arrow keys:**
- `↑` / `↓`: Move cursor through visible items (skip hidden dirs)
- `←`: Collapse parent (if current dir is expanded) or jump cursor to parent dir
- `→`: Expand current dir (if it's a dir; no-op if file)

**Enter:**
- On file: Open file, close sidebar
- On dir: Toggle expand if already selected; if just navigated to it, expand it
  - (Interpretation: repeated Enter on same dir = toggle; navigation onto dir = expand)

**Home / End:** Jump to first/last visible item

**Esc:** Close sidebar, return focus to editor

**Performance:** Tree cached in memory; scanning blocks the render tick (Phase 1.1: lazy-load with thread)

### Fuzzy Picker

**Initial state:** All files shown, cursor on first, input field empty

**Typing:** Each character appended to `input`, live re-filter
- Search algorithm: substring match on `filename + relative_path`
- Scoring: earlier match = higher score; filename prefix beats path substring
- Results sorted by score, then alphabetically

**Backspace:** Remove last character from input, re-filter

**Arrow keys:** Navigate filtered results (↑ / ↓)

**Home / End:** Jump to first/last in filtered results

**Enter:** Open selected file

**Esc:** Close sidebar

**Example:**
```
File structure:
  src/
    main.rs
    app.rs
    render.rs
  tests/
    flow.rs

User types "main":
  Matches: src/main.rs (score: 100, "main" is exact prefix in filename)
  Result: [ src/main.rs ]

User clears and types "app":
  Matches: src/app.rs (filename match)
  Result: [ src/app.rs ]

User types "src/r":
  Matches: src/render.rs (path contains "src/r")
  Result: [ src/render.rs, src/main.rs ] (render ranked higher)
```

### Resize

**Alt-← / Alt-→:**
- `Alt-←`: `sidebar_width -= 2` (min 10)
- `Alt-→`: `sidebar_width += 2` (max 50)
- Width updates immediately; re-renders both sidebar and editor buffer
- Persists across toggle (Ctrl-B → Alt-→ → Esc → Ctrl-B shows wider sidebar)

**Status bar feedback:** Show current width (e.g., "sidebar: 32 cols" or implicit in visual layout)

---

## Layout & Rendering

### Sidebar Render (`render_sidebar`)

```
For tree browser:
  render each item in TreeState.items:
    indent = (item.depth * 2) spaces
    icon = if item.is_dir: "[+]" (collapsed) or "[-]" (expanded) else "  "
    text = item.path.file_name() (truncated if too wide)
    highlight if item == cursor

For fuzzy picker:
  render each item in Sidebar.items (filtered):
    text = relative_path (truncated if too wide)
    highlight if item == cursor
  at top: render input field ">" + input (for visual feedback)
```

### Editor Render
- Same as Phase 0, just narrower (`area.width -= sidebar_width`)

### Status Bar
- If sidebar open: `"[SIDEBAR] Tree/Picker mode | rest of status..."`
- If sidebar closed: `"EDIT | ..."` or `"ACTION | ..."` (unchanged)

---

## `.gitignore` Handling

**Phase 1 scope:**
- If `.gitignore` exists in root, parse it (simple glob matching, not full gitignore spec)
- Hard-coded default ignores: `.git`, `node_modules`, `target`, `__pycache__`, `.egg-info`
- Apply rules: if any rule matches a path, exclude it

**Parsing strategy (lazy choice):**
- Option A: Use `ignore` crate (adds dependency, full spec support)
- Option B: Manual glob matching with simple rules (no dep, handles ~90% of cases)
- Recommendation: Option B for Phase 1 (80% ROI), upgrade to `ignore` in Phase 1.1

**Example rules:**
```
*.o              → exclude *.o files
build/           → exclude build dir and contents
*.log
!important.log   → NOT supported in Phase 1 (negation comes Phase 1.1)
```

---

## Error Handling

- **Scan error (permission denied):** Log to status bar, skip that dir, continue scan
- **Symlink loop:** Detect via visited-set in scan, skip
- **Large tree (1000+ files):** Renders, may lag on low-end terminals (Phase 1.1: threading)
- **Sidebar disappears if all files ignored:** Show "no files" message, close on Esc

---

## Testing & Acceptance

### Unit Tests

**`tree.rs` tests:**
- Build tree on temp dir: verify structure (dirs first, alphabetically)
- `.gitignore` parsing: rules apply correctly
- Ignored paths: `.git`, `node_modules` excluded; normal files included

**`picker.rs` tests:**
- Fuzzy search: "main" → matches `src/main.rs` before `main.rs.bak`
- Path search: "src/app" → matches `src/app.rs`
- Empty search: all files shown
- Backspace: removes last char, results update

**`sidebar.rs` tests:**
- Navigation: cursor moves up/down, wraps at boundaries
- Expand/collapse: toggle_expand adds/removes paths, visible items update
- Selection: selected_path returns current item's path

### Integration Tests

**Flow test additions:**
- Open tree: Ctrl-B → sidebar visible
- Navigate: arrows move cursor
- Expand dir: Right arrow on dir → expands
- Open file: Enter on file → new buffer, sidebar closes
- Open picker: Ctrl-P → picker visible, type "test", Enter → opens matching file
- Resize: Alt-← shrinks, Alt-→ grows, layout recalculates

---

## Phasing & Deferral

### Phase 1 (This Sprint)
✅ Resizable sidebar (10–50 cols)  
✅ Tree browser (navigate, expand/collapse, open files)  
✅ Fuzzy picker (live filter, basename + path search)  
✅ `.gitignore` respect (basic glob matching)  
✅ Hotkeys: Ctrl-B, Ctrl-P, Alt-Left/Right  

### Phase 1.1 (Next Sprint)
⏳ Tree refresh (Ctrl-R or auto-poll mtime)  
⏳ Config file (save sidebar width default, custom ignores)  
⏳ Icons / colors (file/dir visual distinction)  
⏳ Mouse support (click to select/expand)  
⏳ Lazy-load deep trees (background thread)  
⏳ Performance: full `.gitignore` spec with `ignore` crate  

### Out of Scope (Phase 2+)
- New file/directory creation from sidebar
- Rename, delete operations
- File preview on hover
- Tree sync to external editor state (e.g., show current file highlighted)

---

## Success Criteria

- ✅ Sidebar opens/closes with Ctrl-B and Ctrl-P
- ✅ Tree browser navigable: arrows move cursor, Right expands, Left collapses
- ✅ Fuzzy picker filters live as user types
- ✅ Enter opens file and closes sidebar
- ✅ Resize works (Alt-Left/Right), persists in session
- ✅ `.gitignore` applied (if file exists)
- ✅ All Phase 0 functionality preserved (no regression)
- ✅ 70+ tests still passing, new tests cover sidebar behavior
- ✅ No performance regression on 100k-line file editing (sidebar doesn't block ticks)

---

## Implementation Notes (Author)

- **Sidebar state lives in `App`:** keeps lifecycle simple (open ↔ close, not persistent across mode switches)
- **Tree cached at open time:** rebuilds only on explicit request (Phase 1.1)
- **Fuzzy matching is synchronous:** acceptable for typical project sizes (<10k files); threading deferred
- **Layout logic in `render.rs`:** split calculation, sidebar rect, editor rect — all computed per draw
- **Keymap changes:** remove old `Open` command (conflicts), add 8 new sidebar commands (Ctrl-B/P, arrows, Enter, Esc, Alt-Left/Right)
- **Status bar:** minimal change, just adds mode indicator when sidebar open
- **Testing strategy:** unit tests per module, integration flow test for open/navigate/select, no mock filesystem (use temp dirs)

---

## Open Items & Decisions

1. **Resize key conflict:** Alt-Left/Right vs. Ctrl-Left/Right — both could conflict. Alt- chosen (Ctrl- jumps words in editor); confirm no issue.
2. **Tree depth limit:** No hard limit proposed; relies on `.gitignore`. Revisit if large monorepos lag.
3. **UI border:** Sidebar separated from editor by whitespace only (no │ char); simplest render, Phase 1.1 can add visual polish.
4. **Sorting order:** Dirs first, then files, both alphabetically — standard, reviewable.
