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
   them *(shipped: 9 grammars via tree-sitter 0.25 — 6 at first ship,
   python/go/lua added 2026-11-04 — SyntaxState incremental
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
   299 unit + 12 flow + 19 lsp_flow tests, all perf gates green — see README
   "Acceptance (LSP core)"); phase 2 remaining: rename, snippet tab-stops,
   multi-location picker, hover, workspace symbols,
   diagnostics list view, incremental sync, auto-download servers,
   **code actions** (see work order #14); format-on-save split out and
   shipped as work order #17.
6. ✅ **Themes** — shipped: `src/theme.rs` (`parse_color`, `StyleSpec`, `Palette{dark,light}`,
   `SyntaxPalette`, `resolve`/`resolve_in`); `[editor] theme` + `[theme.ui]/[theme.syntax]`
   config keys; full render wiring; 319 unit + 13 flow + 19 lsp_flow tests; pty smoke
   (SGR 38;2;255;0;0 + 48;2;0;255;0 confirmed) — see README "Acceptance (themes)".
Helix-screenshot batch (set 2026-11-04, user-requested after themes; small wins first):
7. ✅ **Current-line highlight** *(shipped as gutter emphasis per design:
   `current_line_no` theme role (dark/light built-in = BOLD+UNDERLINED, no
   color), styled in the gutter loop on the cursor's line; role overridable
   via `[theme.ui]`; 324 unit + 13 flow + 19 lsp_flow)*
8. ✅ **Richer status line** *(shipped: `status_mode` role (built-in REVERSED
   block) on the mode word; dirty `+` marker appended to it (`EDIT+`);
   `LF|CRLF utf-8` format indicator after the language; 326 unit + 13 flow +
   19 lsp_flow; found-and-avoided: rope never contains CR — open() normalizes)*
9. ✅ **Tabs with close ×** *(shipped visual-only per design: active label is
   `[name]+ ×`, joined row centered on the tab line; `tab_active` styles the ×
   span; windowing unchanged; 326 unit + 13 flow + 19 lsp_flow; pty saw the
   U+00D7 bytes live; click wiring deferred to mouse #11 as agreed)*
10. ✅ **Command palette** *(shipped 2026-11-04: C-t opens centered popup, 18-item registry, live subsequence filter, Up/Down/Enter/Esc, key hints reflect remaps, `command_palette` remappable/unbindable, SaveAs reuses existing prompt, dispatch via normal command path; 339 unit + 14 flow + 19 lsp_flow; see README "Acceptance (command palette)")*
11. ✅ **Mouse support** *(shipped 2026-11-05: paint+hit-test share one geometry source (render::layout/tab_line/popup_geom/sidebar_view); click caret (Shift+click extends), tab click activate + ×-close (dirty confirm), sidebar row click = Enter semantics, sidebar border drag resize, wheel 3 rows editor/sidebar/palette, palette row click runs / outside click dismisses; `[ui] mouse` gate default on, Shift-drag keeps native terminal selection; "pane drag" read as sidebar-border drag since panes are #12; 403 unit + 14 flow + 19 lsp_flow; see README "Acceptance (mouse)")*
12. ✅ **Embedded terminal pane** *(shipped 2026-11-06: persistent shell in bottom pane (Ctrl-\ toggle), cwd survives hide/show, Alt-Up/Down resize, PgUp/PgDn capture scrollback (scroll view shipped 2026-11-09 in the scrollback PR; issue #29 closed as a false alarm — stale installed binary), 2000-line ring buffer, nonblocking pump, exception key routing (Ctrl-Q, Ctrl-O, Alt-resize), legacy-terminal caveat (0x1c), device response shim; 496 unit + 14 flow + 19 lsp_flow; see README "Acceptance (embedded terminal pane)")*
13. **Multiline cursor polish** *(parked by user 2026-11-06)*
14. ✅ **LSP code actions** *(shipped 2026-11-10, GitHub issue #9: `Ctrl-.`/`:codeaction`/palette requests `textDocument/codeAction` for the selection-or-caret range with the doc's stored diagnostics re-sent as context (mock echoes the count — round-trip asserted); completion-style dropdown (Tab/BackTab/arrows/Enter/Esc, `KeyOwner::CodeActions`), stale `edit_gen` drop + stale-menu close + close_active purge; generic `WorkspaceEdit` applier parses `changes`+`documentChanges`, resource ops counted & skipped (rename PR will handle them), unopened targets open from disk left dirty, one undo batch per doc, command-only actions rejected with status; chords re-ruled: BufferStart/End → `Ctrl-<`/`Ctrl->` (both kitty spellings pty-probed), legacy `'4'`/`'6'` byte aliases keep buffer bounds so legacy `Ctrl-.` stays BufferEnd — actions colon/palette-only there; format_pump utf16 conversion deduped into `raw_text_edits_to_offsets`; initialize advertises codeAction quickfix/source valueSet; 574 unit + 14 flow + 23 lsp_flow; ruff pty smoke ALL PASS ×3; see README "Code actions (#9)")*
15. ✅ **Global search** *(shipped 2026-11-06, GitHub issue #1: Alt-F search sidebar backed by `rg --vimgrep` streamed via reader-thread+mpsc, 150 ms debounced re-run, 1000-hit cap, Enter/click jumps through apply_goto_location — fully independent of the tree browser; third SidebarMode; ALSO pushed kitty keyboard DISAMBIGUATE_ESCAPE_CODES flags (push/pop at startup, exit, panic hook, suspend/resume) so Shift-Ctrl chords + Ctrl-,/Ctrl-\\ de-collide on supporting terminals; 507 unit + 14 flow + 19 lsp_flow; pty smoke 8/8; see README "Acceptance (global search)")*
16. ✅ **Markdown preview** *(shipped 2026-11-06, GitHub issue #2: `:preview`/palette renders .md or scratch buffers through `glow -s dark -p --width` into a styled buffer — glow's SGR converted to FrameSpan style ranges (colors survive; stale spans dropped when the preview is edited); preview_source tag makes re-preview replace in place, previewing a preview refused, path None so it never overwrites the source; glow-missing → status error; 519 unit + 14 flow + 19 lsp_flow; pty smoke stable ×3 incl. color check; see README "Acceptance (markdown preview)")*
17. ✅ **Format-on-save** *(shipped 2026-11-08, GitHub issue #10 slice: `[editor] format_on_save = true` (default off) — C-s/:w saves first (never blocks on a server), then fires `textDocument/formatting`; response applied via new `Document::apply_text_edits` as ONE undo batch (descending offsets, replace_all_from pattern), file rewritten, status `formatted`; utf16→char conversion + clamping like diagnostics, CRLF newText normalized, stale edit_gen (user typed meanwhile) drops edits; missing/disabled server or null response → plain save; `:wq`/close-saves stay synchronous; initialize now declares `textDocument.formatting`, mock serves it; see README "Acceptance (format on save)")*
18. ✅ **Manual format** *(shipped 2026-11-09: `:format` colon + palette `format_document` (no default chord, CloseBuffer precedent) — formats the active buffer WITHOUT saving: request core extracted as `request_format(idx, resave)`, manual path uses resave=false so edits land buffer-only (disk never written, buffer stays dirty, one undo reverts), save path byte-identical; no-path/no-lang → "no language server for this file", disabled/missing server → "no language server for {lang}"; pending_format gained the resave flag; format_pump honors it; 543 unit + 14 flow + 20 lsp_flow; pty smoke 7/7 stable ×3 incl. disk-untouched + dirty-exit-confirm checks)*
19. ✅ **Completion live filtering** *(shipped 2026-11-10, from "review the completion functionality" — plumbing was healthy: root cause was plat ignoring the LSP client-filtering contract (gopls/rust-analyzer deliberately return whole-scope client-filterable lists) plus any keystroke closing the menu with no re-request. `completion_pump` retains items whose label subsequence-matches the identifier word before the caret (reuses `picker::subsequence_score`); no word at caret → server order kept; typing/Backspace with the menu open inserts the key AND re-requests completion so the reopened menu carries fresh items AND fresh `textEdit` ranges (re-filtering an old response would splice at pre-typing offsets — stale-range trap); nothing matches → no menu. Server note: ruff's server advertises NO completion provider (returns null) — python users need pylsp for completion (pairs with `[formatting]` below, #20); `initialize` left alone (rust-analyzer answers completion without declared caps — headless probe). Mock flow test re-anchored `alp`→`a`; 582 unit + 14 flow + 25 lsp_flow; real pylsp+gopls pty smoke ALL PASS ×3; see README completion paragraph)*
20. ✅ **External formatters (`[formatting]` table)** *(shipped 2026-11-10: split LSP intelligence from formatting — `[formatting]` per-language CLI formatter commands with the same string/array/false semantics as `[lsp]` (value parsing shared via extracted `lsp_value_command`); NO built-in defaults, absent key → `textDocument/formatting` on the LSP server unchanged. `format_document`/`maybe_format_after_save` consult `formatter_for(lang)` first: whole buffer piped stdin→stdout on a worker thread (+ stdin-feeder thread: UI never blocks, no pipe deadlock on big buffers), stdout replaces the buffer as ONE undo step (`Document::edit`), stale `edit_gen` while the formatter runs drops the result, exit≠0 → `formatter failed: <stderr>` status buffer intact, CRLF output normalized to the LF rope; `false` → manual hint / on-save silent skip. Pairing example: `python = "pylsp"` + `python = "ruff format -"`; 587 unit + 14 flow + 25 lsp_flow; real-ruff pty smoke ALL PASS ×3 (palette format buffer-only + disk untouched, format_on_save disk rewrite, broken-formatter hint); see README "Config `[formatting]` table")*

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
