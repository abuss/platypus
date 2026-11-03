# Platypus (`plat`) — Plan

A terminal text editor where **edit mode is home**: typing always goes straight
into the buffer. **Esc** enters a slim *action mode* — an optional shortcut for
moving, selecting, searching and commands, used to reduce keystrokes and finger
stress. No vim grammar, arrow-key movement.

The mode model is the product. Everything else is a menu picked from later.

## Stack

- **Rust**: ratatui + crossterm, ropey, tree-sitter, TOML config
- **Embedded terminal**: portable-pty + vt100 + tui-term
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

## Embedded terminal overlay — **decided in scope (2026-11-03)**

Resolves the contradiction in `ideas.md`, which listed both "Terminal Panel" as
a feature and "Built-in Terminal — out of scope" as a non-feature. It is in
scope: a small overlay terminal for one-off commands (builds, `rg`, `man`),
not a tmux replacement. Suspend (`Ctrl-O`) stays the path for programs that
want the real terminal (full-screen TUIs, editors, lazygit).

- `portable-pty` (spawn real shell) + `vt100` (parse/scrollback) +
  `tui-term` (ratatui widget). ~150–250 lines given overlay machinery.
- Gotchas: key→bytes forwarding table; resize both parser and PTY; SIGHUP the
  child before closing the pty writer; scrollback via `Parser::new` third arg.
- **Open design question**: Esc belongs to the embedded terminal (vim inside
  it). The escape-hatch key must be decided in the spec — `Ctrl-\` is
  reserved-unbound today and is the leading candidate.
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
- ⏭ **Embedded terminal overlay** + command pipe-through (section above)
- ⏭ Search & replace (regex, live preview)
- ⏭ TOML config + remappable keymaps (retires the `PLAT_*` env switches)
- ⏭ Tree-sitter syntax highlighting — last, because it touches rendering and
  wants config settled first
- Sidebar follow-ups (fold into the config task, not a phase of their own):
  re-scan after external file changes; `←` jumps to the parent dir (the
  Phase 1 spec asked for this and the build shipped a toggle instead);
  width + ignore patterns from config.

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
- Esc-vs-terminal-overlay escape hatch key
