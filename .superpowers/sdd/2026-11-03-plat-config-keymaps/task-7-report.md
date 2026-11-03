# Task 7 Report: Tree arrows — `→` expand, `←` collapse-else-parent

## Status
✅ **COMPLETE** - All tests passing (164 unit + 8 flow), no warnings, fully integrated.

## Implementation Summary

### Step 1: TDD RED — Failing Tests Added
Added two failing test cases per brief:
1. **keymap test**: `sidebar_owns_the_keyboard_when_open()` updated to expect:
   - `KeyCode::Right` → `Command::SidebarExpand` 
   - `KeyCode::Left` → `Command::SidebarCollapseOrParent`

2. **sidebar test**: `left_arrow_collapses_then_walks_to_parent()` verifies:
   - `expand_path()` adds a dir to the expanded set
   - `collapse_or_parent()` on a file jumps cursor to parent row
   - `collapse_or_parent()` on an expanded dir collapses it

### Step 2: Implementation
**keymap.rs** (10 lines changed):
- Replaced `SidebarToggleExpand` variant with two: `SidebarExpand`, `SidebarCollapseOrParent`
- Split `KeyCode::Left | KeyCode::Right` match arm into separate arms in `lookup_sidebar()`
- Updated test expectations

**sidebar.rs** (43 lines added):
- Added `pub fn expand_path(&mut self, path: &PathBuf)` — adds path to expanded set, clamps cursor
- Added `pub fn collapse_or_parent(&mut self, path: &PathBuf)`:
  - If path is expanded: remove from set, clamp cursor
  - Else: find parent path in `visible_items()` and move cursor there (no-op if absent)
- Added helper `fn sb_item(path, is_dir, depth)` for test construction
- Added full integration test `left_arrow_collapses_then_walks_to_parent()`

**app.rs** (11 lines changed):
- Replaced single `SidebarToggleExpand` handler with two arms:
  - `SidebarExpand`: calls `expand_path()` on selected dir (guards `is_dir()`)
  - `SidebarCollapseOrParent`: calls `collapse_or_parent()` on any selected item
- Removed unnecessary `is_dir()` guard on collapse arm (works on files and dirs)

### Step 3: TDD GREEN — Full Test Suite

```
$ cargo test
   Compiling plat v0.1.0
    Finished `test` profile
     Running unittests src/lib.rs

running 164 tests
test result: ok. 164 passed; 0 failed; 1 ignored

     Running unittests src/main.rs
test result: ok. 0 passed; 0 failed

     Running tests/flow.rs
running 8 tests
test result: ok. 8 passed; 0 failed

   Doc-tests plat
test result: ok. 0 passed; 0 failed
```

**Result**: ✅ All 164 unit + 8 flow tests pass. Zero warnings. Clean build.

### Files Modified
- `src/keymap.rs` — Command enum, lookup_sidebar(), keymap test
- `src/sidebar.rs` — Sidebar impl (expand_path, collapse_or_parent), sidebar test
- `src/app.rs` — handle_sidebar_command() arms for new variants

### Grep Verification
- `SidebarToggleExpand` variant: DELETED ✅ (no remaining matches in src/)
- All callers updated: app.rs arms now dispatch to correct handlers
- No compilation warnings or errors

### Design Notes
- `collapse_or_parent()` gracefully handles missing parent: if parent not in `visible_items()` (e.g., parent collapsed or root file), cursor does not move (no-op). Brief specified this: "no-op if absent".
- Both methods clamp cursor after mutation to stay within bounds of `visible_items()`.
- `expand_path()` is simpler than `toggle_expand_path()`: always inserts, idempotent on repeated calls (HashSet insert is safe).
- Cursor guard logic matches existing `toggle_expand_path()` pattern for consistency.

## Self-Review Checklist
✅ TDD: RED (failing tests added) → GREEN (all tests pass)  
✅ Command variants created and used correctly  
✅ SidebarToggleExpand fully deleted (grep confirms)  
✅ New methods added to Sidebar with correct signatures  
✅ Test helper `sb_item()` reuses existing test pattern  
✅ App arms dispatch correctly to new methods  
✅ Phase 1 test expectations updated per brief  
✅ Full test suite green (164 + 8)  
✅ Zero warnings or errors  
✅ Consistent with codebase style and patterns  

## Concerns
None. The implementation is minimal, focused, and complete. All tests pass. No regressions.

---
**Commit**: Ready for `git commit -m "Task 7: Tree arrows — Right→expand, Left←collapse-or-parent"`
