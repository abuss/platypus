# Platypus (`plat`) — Plan

A terminal text editor where **edit mode is home**: typing always goes straight
into the buffer. **Esc** enters a slim *action mode* — an optional shortcut for
moving, selecting, searching and commands, used to reduce keystrokes and finger
stress. No vim grammar, arrow-key movement.

The mode model is the product. Everything else is a menu picked from later.

## Stack

- **Rust**: ratatui + crossterm, ropey, tree-sitter, TOML config
- **Embedded terminal**: portable-pty + vt100 (behind a small `Emulator` trait)
- **Strategy**: patch a Helix fork (initial-mode + slim keymap) and daily-drive
  it ~2 weeks to validate the mode model. If it works and Helix's internals
  fight normal-mode-centric assumptions, do a small rewrite (~3–5k lines for
  Phase 0) using Helix as architecture reference, not base.
  - Helix upstream explicitly rejected insert-mode-as-home (PR #13593) and
    treats long insert-mode sessions as a known-buggy path — that rejection is
    platypus's reason to exist, and the maintenance surface any fork owns.

## Motion model (action mode)

- Arrow keys for movement; no hjkl, no verb-first vim grammar.
- Keep the parts of "vim grammar" that carry the speed, selection-first:
  - textobjects (inside quotes / parens / word) as select-expand keys
  - word jumps (`w`/`b` equivalents)
  - `f`-jump, search-as-selection
- Validate arrow-only ergonomics in Phase 0; add word-jump keys if wrist
  travel hurts.

## Git = external tools

- **Suspend-to-terminal keybinding** → lazygit (stage, commit, blame,
  conflicts), difftool for diffs. One primitive, no git UI subsystem.
- Editor-side work: file-watcher + reload-on-change + dirty-buffer conflict
  prompt. Undo history survives reload.
- Optional later: read-only gutter diff markers, branch in status bar.

## Embedded terminal pane — **in scope, design agreed (2026-11-03)**

Resolves the contradiction in `ideas.md`, which listed both "Terminal Panel" as
a feature and "Built-in Terminal — out of scope" as a non-feature. Bottom pane
running a **persistent interactive shell** (`$SHELL` else `/bin/sh`, cwd =
plat's), not a throwaway popup: cd, history, tab completion and scrollback
survive toggling the pane. Suspend (`Ctrl-O`) stays the path for full-screen
TUIs that want the real terminal (lazygit, git commit $EDITOR).

- **Pane**: bottom, above the status bar; editor gets the rows above. Default
  height 10, clamp `3 ..= (rows-2)/2`, Alt-Up/Down resize. `Ctrl-\` toggles
  *visibility* (process + scroll position survive hiding); one pane per plat
  instance; status bar gains `shell: <dir>` / `shell exited`.
- **Keys**: while visible the pane owns everything except `Ctrl-\` (toggle),
  `Ctrl-Q` (quit), `Ctrl-O` (suspend), Alt-Up/Down (resize) and PgUp/PgDn
  (scrollback, capacity 2000; printable input snaps back to live first).
  Shell loses SIGQUIT. The key→bytes table is ours (~45 lines); arrow form
  (SS3 vs CSI) follows `application_keypad()`.
- **Parser**: `vt100` behind a small `Emulator` trait (write / resize / draw /
  cursor / scroll / application_keypad) so libghostty-vt can swap in later
  without Zig in the default build. vt100 generates no device responses; a
  ~20-line shim in the reader path answers `ESC[6n`, `ESC[5n`, `ESC[c`,
  `ESC[>c`.
- **Deps**: `portable-pty` + `vt100`, both pure Rust. `tui-term` rejected —
  its controller is oneshot-commands-only, a persistent shell would be
  hand-rolled anyway.
- Command pipe-through pairs with it: selection → stdin → replace with stdout.

## Roadmap

### Phase 0 — MVP (the hypothesis test) — **shipped**
- Open/save a file; rope buffer; render; type in edit mode
- Esc → action mode: arrows, selection, find, undo/redo
- Suspend binding + reload-on-change
- Quit. Live in it for two weeks. ~3–5k lines. *(~1.6k lines shipped; daily
  driving since — `PUNCHLIST.md` is the tuning channel.)*

### Phase 1 — **in progress**
- ✅ File sidebar: tree browser (Ctrl-B) + fuzzy picker (Ctrl-P), resizable
  10–50 cols, gitignore-lite *(pulled forward from Phase 2; spec, plan and
  123 tests under `docs/superpowers/`)*
- ✅ Line-number gutter (`PLAT_LINENO`)
- ⏭ **Embedded terminal pane** + command pipe-through (design section above)
- ⏭ Search & replace (regex, live preview)
- ⏭ TOML config + remappable keymaps (retires the `PLAT_*` env switches)
- ⏭ Tree-sitter syntax highlighting — last, because it touches rendering and
  wants config settled first
- Sidebar follow-ups (fold into the config task, not a phase of their own):
  re-scan after external file changes; `←` jumps to the parent dir (the
  Phase 1 spec asked for this and the build shipped a toggle instead);
  width + ignore patterns from config; `apply_resize()` clamps scroll against
  full width instead of width-minus-sidebar; browsable symlinked dirs; real
  fuzzy scoring in the picker; Ctrl-F while the sidebar is open.

### Phase 2
- LSP (its own phase: subsystem-sized) — diagnostics, completion,
  goto-definition, rename, LSP snippets, format-on-save
- Mouse support (click to place the cursor, sidebar click/expand, pane drag)
- Themes, multiline cursor polish

### Phase 3 — if ever
- Git gutters / branch in status bar
- Bookmarks, breadcrumbs, resizable panes
- **Lua plugin system** (future enhancement — moved out of core phases;
  biggest cost, least justified for a personal editor; revisit only when
  config + keymaps measurably fall short)

## Explicitly cut
- Lua plugin **package manager** (deleted, not deferred)
- Snippet engine (LSP snippets cover it)
- Macro recording
- Merge-conflict UI, blame UI, staging UI (lazygit does it better)
- Vim plugin porting; VimScript/Emacs Lisp

## Success criteria
- Hybrid mode model feels faster than plain editing and than Helix, daily
- Responsive on 100k+ line files; instant startup (ropey gives this)
- TOML config ~50 lines covers all customization

## Open items
- Name: **`plat`** (was `ptp`, which collides with an existing terminal
  planner on PATH)
