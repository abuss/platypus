# Workspace Symbols Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two LSP-backed sidebar modes for navigating code symbols: file symbols (Ctrl-O) and workspace symbols (Shift-Ctrl-O), both jumping to definitions with sidebar remaining open.

**Architecture:** Extend `SidebarMode` enum with two new variants (`FileSymbols`, `WorkspaceSymbols`). Add corresponding state structs holding symbol list, LSP data, and query (for workspace). LSP integration sends `textDocument/documentSymbol` (file) and `workspace/symbol` (workspace, debounced) requests. Rendering adds symbol rows with Unicode kind icons and file location. Tree-sitter fallback ensures symbols appear without LSP.

**Tech Stack:** Rust, serde_json (LSP marshaling), tree-sitter (fallback parsing), existing LSP client infrastructure

**Spec:** `docs/superpowers/specs/2026-11-14-workspace-symbols-design.md`

## Global Constraints

- **Keybindings:** `Ctrl-O` (file symbols), `Shift-Ctrl-O` (workspace symbols); both also in command palette
- **Jump behavior:** Jump to symbol definition, keep sidebar open (continuous browsing)
- **Filter strategy:** Server-side LSP filtering (per keystroke)
- **Result limit:** First N (default 100) with truncation indicator
- **Symbol source:** Tree-sitter primary, LSP fallback if available
- **Debounce:** 200ms on workspace symbol queries to avoid request spam
- **Display format:** `<icon> <name> @ <file>:<line>` with Unicode kind icons
- **Backward compatibility:** No breaking changes to existing sidebar modes or command infrastructure
- **Testing:** All tests pass before PR; no ignored tests added

## Review Focus

These five input classes and failure modes are most likely to cause problems if not covered:

1. **No active buffer when Ctrl-O pressed:** Status message shown, sidebar not opened
   - Test in Task 2: `test_file_symbols_no_active_buffer`

2. **LSP server unavailable/slow response:** Tree-sitter fallback symbols appear immediately
   - Test in Task 3: `test_file_symbols_tree_sitter_fallback`

3. **Workspace symbol query with no matches:** Show "No symbols found" message
   - Test in Task 4: `test_workspace_symbols_empty_results`

4. **Rapid typing in workspace mode:** Only one request every 200ms (debounce working)
   - Test in Task 4: `test_workspace_symbol_query_debounce`

5. **Jump to symbol in different file:** File opens, cursor moves to correct location, sidebar stays open
   - Test in Task 5: `test_jump_workspace_symbol_different_file`

---

## File Structure

**Create:**
- `src/symbols.rs` — Symbol data types and LSP parsing (new file)
- `tests/symbols.rs` — Symbol parsing and rendering tests (new file)

**Modify:**
- `src/sidebar.rs` — Add `FileSymbolState`, `WorkspaceSymbolState`, update `SidebarMode` enum
- `src/keymap.rs` — Add `FileSymbols`, `WorkspaceSymbols` commands; add Ctrl-O, Shift-Ctrl-O keybindings
- `src/app.rs` — Add symbol request handlers, query processing, jump-to-symbol logic
- `src/lsp/mod.rs` — Add request ID tracking for file/workspace symbol requests
- `src/lsp/mock.rs` — Add mock handlers for `textDocument/documentSymbol` and `workspace/symbol`
- `src/render.rs` — Add rendering for FileSymbols and WorkspaceSymbols sidebar modes
- `src/lib.rs` — Export `symbols` module

**No deletion or major refactoring needed.** All changes are additive or narrow modifications to existing structures.

---

## Implementation Strategy

This plan decomposes into 8 independent tasks that build on each other:

1. **Task 1**: Symbol data structures + LSP parsing
2. **Task 2**: Sidebar state + file symbols infrastructure  
3. **Task 3**: File symbols LSP response parsing + tree-sitter fallback
4. **Task 4**: Workspace symbols with server-side filtering & debouncing
5. **Task 5**: Symbol jump navigation + sidebar rendering
6. **Task 6**: Keybindings + command palette integration
7. **Task 7**: Integration tests & edge case coverage
8. **Task 8**: Documentation & final verification

**Expected to ship as 2–3 PRs:**
- PR 1: Tasks 1–2 (data structures + file symbols infrastructure)
- PR 2: Tasks 3–4 (file & workspace symbols implementation)
- PR 3: Tasks 5–8 (rendering, navigation, keybindings, tests, docs)

**Estimated LOC**: 800–1200 (new + modified)  
**Estimated effort**: 8–12 engineering hours

---

## Task Details (See Design Spec for Full Implementation)

For complete step-by-step implementation details (test code, function signatures, mock implementations, etc.), refer to the full design document:

**Spec file:** `docs/superpowers/specs/2026-11-14-workspace-symbols-design.md`

Each task in this plan includes:
- Exact files to create/modify
- Interfaces (what each task produces for the next)
- Step-by-step checklist with specific test names and commit messages
- Code examples and signatures

### Quick Task Summary

| Task | Deliverable | Key Functions |
|------|-------------|----------------|
| 1 | Symbol types + parsing | `LspSymbol`, `parse_file_symbols()`, `parse_workspace_symbols()`, `symbol_kind_icon()` |
| 2 | Sidebar infrastructure | `FileSymbolState`, `SidebarMode::FileSymbols`, request tracking |
| 3 | LSP response handling | `on_file_symbols_response()`, `load_file_symbols_from_tree_sitter()` |
| 4 | Workspace queries | `WorkspaceSymbolState`, `SidebarMode::WorkspaceSymbols`, `on_workspace_symbol_query()`, debouncing |
| 5 | Navigation + rendering | `jump_to_symbol()`, symbol row rendering, truncation indicators |
| 6 | Keybindings | `Command::FileSymbols`, `Command::WorkspaceSymbols`, Ctrl-O, Shift-Ctrl-O, palette entries |
| 7 | Integration tests | Full flow tests, Review Focus edge case coverage |
| 8 | Documentation | README, REFERENCE.md, feature list |

---

## Execution Path

**Subagent-driven approach recommended:**
- Fresh subagent per task (independent implementation)
- Fresh reviewer per task before next one starts
- One whole-branch review at the end
- Most thorough; allows parallelization of independent tasks

**Native approach available:**
- Current session implements all tasks sequentially
- Single reviewer on final branch
- Faster; requires careful attention to interfaces between tasks

Which approach would you prefer? Subagent-driven testing is more thorough but slower; native execution is faster but relies on careful adherence to the interfaces defined in each task.

