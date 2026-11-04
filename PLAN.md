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

## Embedded terminal pane — **in scope, design agreed (2026-11-03); work order #8**

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

### Phase 1
- ✅ File sidebar: tree browser (Ctrl-B) + fuzzy picker (Ctrl-P), resizable
  10–50 cols, gitignore-lite *(pulled forward from Phase 2; spec, plan and
  123 tests under `docs/superpowers/`)*
- ✅ Line-number gutter (`PLAT_LINENO`)

### Work order — set 2026-11-03

1. ✅ **Search & replace** (regex, live preview) *(shipped: regex live-search on
   every keystroke, Tab ⇄ replacement field, `y`/`n`/`a`/`e`/Esc walkthrough,
   batched undo for `a`; 144 tests; pty smoke + 100k-line perf gate — see
   README acceptance)*
2. ✅ **TOML config + remappable keymaps** (retires the `PLAT_*` env switches;
   folds in the sidebar follow-ups: re-scan after external file changes, `←`
   jumps to parent dir, width + ignore patterns from config, `apply_resize()`
   scroll clamp with sidebar open, browsable symlinked dirs, real fuzzy
   scoring, Ctrl-F while sidebar is open) — 175 unit + 9 flow tests; pty smoke
   + exit-1 check; see README "Acceptance (config + keymaps)"
3. ✅ **Tree-sitter syntax highlighting** — touches rendering, wants config
   settled first; colors hardcoded in the first pass, themes at #6 retrofit
   them *(shipped: 6 grammars via tree-sitter 0.25, SyntaxState incremental
   parse + viewport-range query, four mutation paths, render paint, frame
   cache; 194 unit + 10 flow tests; pty smoke + perf report — see README
   "Acceptance (syntax highlighting)"; build ✅ ~24ms; mid-file edit ✅ ~3.6ms; warm ✅ ~1µs —
   windowed parse (view ± 2000 lines) keeps keystroke cost O(viewport))*
4. **Status bar upgrades** (scope at design time — candidates: line:col,
   document %, read-only git branch) *(shipped: left gains git(branch) —
   per-path cached lookup incl. negatives, detached HEAD → short SHA — and
   detected language; right-aligned line:col / "N selected" / Top·42%·Bot;
   narrow terminals keep the right side pinned; 199 unit + 11 flow tests +
   pty smoke — see README "Acceptance (status bar)"; follow-up: top tab row
   for multi-buffer sessions + Alt-h/Alt-l buffer cycling with wrap-around —
   see README "Acceptance (tab row & cycling)")*
5. **LSP** — **phase 1 shipped** (diagnostics + completion + goto-definition,
   294 unit + 12 flow + 19 lsp_flow tests, all perf gates green — see README
   "Acceptance (LSP core)"); phase 2 remaining: rename, format-on-save,
   snippet tab-stops, multi-location picker, hover, workspace symbols,
   diagnostics list view, incremental sync, auto-download servers.
6. **Themes**
7. **Mouse support** (click to place cursor, sidebar click/expand, pane drag)
8. **Embedded terminal pane** — design already agreed, see section above;
   spec + plan when it comes up
9. **Multiline cursor polish**

Later — if ever: git gutters / branch in status bar, bookmarks, breadcrumbs,
resizable panes, **Lua plugin system** (biggest cost, least justified for a
personal editor; revisit only when config + keymaps measurably fall short)

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
