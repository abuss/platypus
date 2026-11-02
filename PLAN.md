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

## Embedded terminal overlay

- `portable-pty` (spawn real shell) + `vt100` (parse/scrollback) +
  `tui-term` (ratatui widget). ~150–250 lines given overlay machinery.
- Gotchas: key→bytes forwarding table; resize both parser and PTY; SIGHUP the
  child before closing the pty writer; scrollback via `Parser::new` third arg.
- **Open design question**: Esc belongs to the embedded terminal (vim inside
  it). Escape-hatch key (e.g. `Ctrl-\`) must be decided before finalizing the
  keymap.
- Suspend remains the path for programs wanting the real terminal.

## Roadmap

### Phase 0 — MVP (the hypothesis test)
- Open/save a file; rope buffer; render; type in edit mode
- Esc → action mode: arrows, selection, find, undo/redo
- Suspend binding + reload-on-change
- Quit. Live in it for two weeks. ~3–5k lines.

### Phase 1
- Tree-sitter syntax highlighting
- Search & replace (regex, live preview)
- TOML config + keymaps
- Terminal overlay (portable-pty trio); command pipe-through
  (selection → stdin → replace with stdout)

### Phase 2
- LSP (its own phase: subsystem-sized) — diagnostics, completion,
  goto-definition, rename, LSP snippets, format-on-save
- Tree sidebar (collapsible, mouse + keyboard)
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
