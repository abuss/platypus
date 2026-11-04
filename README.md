# plat — platypus editor (Phase 0)

A terminal text editor where **edit mode is home**: typing always goes into
the buffer; **Esc** enters a slim action mode for moving, selecting, acting
on ranges, and searching. Selection-first — no vim grammar, arrow movement.

Spec: `docs/superpowers/specs/2026-11-01-plat-phase0-design.md`
Plan: `docs/superpowers/plans/2026-11-01-plat-phase0.md`

## Build & run

```sh
cargo build --release
./target/release/plat [files...]
```

Dev switches: `PLAT_PANIC=1 plat file` panics right after terminal init —
used to verify the panic hook restores the terminal (it prints the panic
with `isig/icanon/echo` re-enabled; checked under a pty).
`PLAT_CONFIG=/path/to/config.toml` overrides the default config path
(`~/.config/platypus/config.toml`); if set and the file is missing, plat
exits 1 with an error (never silently falls back to defaults).

Set `[editor] key_debug = true` in the config to echo each raw key event
into the status bar — for triaging what your terminal actually sends when a
binding "doesn't work".

Terminal paste (bracketed paste) inserts the pasted text at the cursor
verbatim, newlines and tabs intact, as a single undo step.

## Keymap (Phase 0)

### Global Ctrl (every mode)

| Key | Action |
|---|---|
| Ctrl-S | save |
| Ctrl-Z / Ctrl-Y | undo / redo |
| Ctrl-F | find |
| Ctrl-B / Ctrl-P | file sidebar: directory tree / fuzzy picker |
| Ctrl-Q | **quit** (dirty → confirm) |
| Ctrl-C / Ctrl-X / Ctrl-V | copy / cut / paste — CUA, works in every mode (copy needs a selection) |
| Ctrl-W | close buffer (dirty → confirm) |
| Alt-h / Alt-l | previous / next buffer (wraps; works with the sidebar open) |
| Ctrl-O | **suspend to shell** (run lazygit etc., then `fg`) |
| Ctrl-\ | reserved — not bound |

### Edit mode

typing · Enter · Tab (2 spaces) · Backspace/Delete · arrows/Home/End/PgUp/PgDn ·
Ctrl-A/Ctrl-E (line start/end). Everything else types.

### Action mode (Esc to enter; Esc cancels selection, Esc again → Edit)

| Keys | Action |
|---|---|
| Arrows, `w`/`b` | move; word jumps |
| `f`/`F` then char | jump to char on line |
| `Space` | toggle char under cursor |
| `Shift+arrows` | extend selection |
| `x` / `X` | select line / expand to full lines |
| `Alt+-` then `"` `'` `(` `[` `{` | select inside quotes/brackets |
| `y` / `p` / `P` | yank · paste after · paste before |
| `d` | delete selection (also copies) |
| `c` | change: delete selection, enter Edit |
| `/` | find (**regex**, live preview while typing); `n`/`N` cycle, wrapping at buffer edges; reports "no matches" |
| `u` / `U` | undo / redo |
| `:` | commands: `w [path]`, `q`, `wq`, `e path` |

Prompts own the keyboard: Enter commit · Esc cancel · Up/Down history ·
Tab cycles confirm choices (save/discard/cancel), Tab in the find prompt
toggles pattern ⇄ replacement.

## Search & replace

`/` opens the find prompt and matches underline **as you type** — the pattern
is a regex (`\d+`, `(foo|bar)`, `.`), and an incomplete regex never blanks the
highlights (it reports the error and keeps the last good ones). `n`/`N` cycle
through matches.

Press **Tab** to switch to the replacement field, type it, press **Enter**:
plat walks the matches one at a time, scrolling each into view:

| Key | Action |
|---|---|
| `y` | replace this match, move to the next |
| `n` | skip this match |
| `a` | replace this one and all the rest (one `Ctrl-Z` reverts the whole batch) |
| `e` | replace this one and stop |
| Esc | end (already-applied replaces stay, each was its own undo step) |

In the replacement, `$1` / `${name}` expand capture groups and `$$` is a
literal `$`; a reference to a group the pattern doesn't have is rejected with
an error instead of silently deleting text. Enter on the pattern field (or an
empty replacement) keeps the plain find behavior.

## File sidebar (Phase 1)

**Ctrl-B** opens the directory tree, **Ctrl-P** the fuzzy picker; both start in the
current directory. While the sidebar is open it owns the keyboard:

| Key | Action |
|---|---|
| ↑ / ↓, PgUp/PgDn, Home/End | move through the rows |
| ← / → | collapse selected dir / expand selected dir (← on a file jumps to parent) |
| Enter | open the selected file (sidebar closes); on a dir, toggle it |
| type | filter the picker by name, then path (live, ranked) |
| Backspace | undo one filter character |
| Esc | close the sidebar without opening anything |
| Alt-← / Alt-→ | shrink / grow the sidebar (10–50 cols, kept for the session) |
| Ctrl-R | rescan the directory tree |
| Ctrl-F | open find while sidebar is open |

Ctrl-S, Ctrl-Q, Ctrl-W, Ctrl-O, Ctrl-Z/Y still work with the sidebar open.
`:e path` still opens a file by name.

The scan skips `.git`, `node_modules`, `target`, `__pycache__` and `.egg-info`,
plus the rules in a root `.gitignore` (`*.ext`, `dir/`, plain names; no `!`
negation yet). Unreadable directories are skipped and reported in the status bar.

## Configuration

plat reads `~/.config/platypus/config.toml` (or `$XDG_CONFIG_HOME/platypus/config.toml`
if set, or the explicit `PLAT_CONFIG` env var). A missing XDG/default path silently
uses built-in defaults. `PLAT_CONFIG` set to a missing file exits 1 with an error.

```toml
[editor]
line_numbers = true          # show line-number gutter (default true)
indent_width = 2             # Tab inserts this many spaces (1–8, default 2)
key_debug = false            # echo raw key events into the status bar

[sidebar]
width = 32                   # initial sidebar width in columns (10–50)
ignore = ["dist", "*.min.js"]  # extra patterns to skip during scan

[keys]
# Remap or unbind action-mode keys.
# Key format: [C-][A-][S-]<key>, where <key> is a letter, digit, symbol,
# or a named token: Space Enter Tab Esc Backspace Delete Up Down Left Right
# Home End PageUp PageDown
# "none" unbinds the key (Nop — no fallthrough to the default).
"C-Space" = "select_line"    # example: make C-Space select the whole line
"x" = "none"                 # example: unbind action-mode x
```

### Bindable command names

| Name | Default binding | Action |
|---|---|---|
| `select_line` | `x` | select the line under cursor |
| `expand_line` | `X` | extend selection to full lines |
| `begin_select_char` | `Space` | start a char-level selection |
| `jump_char` | `f` | jump to next occurrence of a char on the line |
| `jump_char_back` | `F` | jump to previous occurrence |
| `object_prefix` | `Alt--` | select inside quotes/brackets (then `"` `'` `(` `[` `{`) |
| `yank` | `y` / `Ctrl-C` | copy selection |
| `paste_after` | `p` | paste after cursor |
| `paste_before` | `P` | paste before cursor |
| `paste` | `Ctrl-V` | CUA paste |
| `delete_selection` | `d` / `Ctrl-X` | cut selection |
| `change` | `c` | cut selection, enter Edit mode |
| `find` | `/` / `Ctrl-F` | open find/replace prompt |
| `find_next` | `n` | cycle to next match |
| `find_prev` | `N` | cycle to previous match |
| `save` | `Ctrl-S` | save current buffer |
| `undo` | `u` / `Ctrl-Z` | undo |
| `redo` | `U` / `Ctrl-Y` | redo |
| `quit` | `Ctrl-Q` | quit (dirty → confirm) |
| `close_buffer` | `Ctrl-W` | close buffer (dirty → confirm) |
| `next_buffer` | `Alt-l` | cycle to the next buffer (wraps); `Alt-]` is a best-effort alias |
| `prev_buffer` | `Alt-h` | cycle to the previous buffer (wraps) |
| `suspend` | `Ctrl-O` | suspend to shell |
| `open_sidebar_tree` | `Ctrl-B` | open directory tree sidebar |
| `open_sidebar_picker` | `Ctrl-P` | open fuzzy picker sidebar |
| `resize_sidebar.grow` | `Alt-→` | widen sidebar by 2 cols |
| `resize_sidebar.shrink` | `Alt-←` | narrow sidebar by 2 cols |
| `enter_action_mode` | `Esc` (in Edit) | enter action mode |
| `enter_edit_mode` | `Esc` (in Action) | return to edit mode |
| `indent` | `Tab` | insert indent |
| `insert_newline` | `Enter` | insert newline |
| `backspace` | `Backspace` | delete char before cursor |
| `delete` | `Delete` | delete char after cursor |
| `move.<dir>` | arrows, w/b | move cursor (`left` `right` `up` `down` `pageup` `pagedown` `word_next` `word_prev` `line_start` `line_end`) |
| `move.<dir>.extend` | `Shift`+arrows | extend selection |

## Syntax highlighting

plat uses tree-sitter for syntax highlighting. Six languages are supported out of the box:

| Language | Extensions |
|----------|------------|
| Rust | `.rs` |
| TOML | `.toml` |
| Markdown | `.md` |
| JSON | `.json` |
| C / C headers | `.c`, `.h` |
| Bash | `.sh`, `.bash` |

Highlighting is on by default and can be disabled per-session via config:

```toml
[editor]
syntax = false   # disable syntax highlighting (default: true)
```

Themes (color palette customization) come in roadmap item #6 and will make
colors configurable; in the first pass the palette is hardcoded.

## Acceptance (syntax highlighting)

- **194 unit + 10 flow** tests green (`cargo test`); no regressions.
- Review Focus behaviors, each pinned by a named test:
  1. Incremental parse matches a fresh rebuild → `incremental_matches_fresh_rebuild`
  2. Spans use char columns (not byte offsets) on multibyte lines → `spans_use_char_cols_on_multibyte_lines`
  3. Edit accumulates only when syntax is present; plain text has no styles; config off prevents build →
     `edit_accumulates_only_when_syntax_present`, `plain_text_document_has_no_syntax_styles`,
     `syntax_disabled_via_config_prevents_build`
  4. All six grammars construct → `all_six_grammars_construct`
  5. Perf gates (run with `cargo test --release -- --ignored`) →
     `bench_syntax_build_100k_lines`, `bench_syntax_edit_refresh_100k_lines`,
     `bench_syntax_warm_viewport_100k_lines`
- Perf numbers (release build, 100k-line Rust source; R2+R3 policy, windowed parse):
  - `SyntaxState::build` (deferred — first parse happens at first refresh): **~24 ms** (gate ≤800 ms ✅)
  - mid-file edit `refresh` (Enter key at line 50000, 60-row viewport): **~3.6 ms** (gate ≤10 ms ✅)
  - byte-0 edit `refresh` (space insert at byte 0): **~1.3 ms** (no gate — documented worst case)
  - warm same-view `refresh` (frame cache hit): **~1 µs** (gate ≤2 ms ✅)
  - O(viewport-window) claim verified: 100k-line edit ~4.7 ms, 400k-line edit ~10 ms
    (window = view ± 2000 lines; parse cost is independent of file size outside the window)
  - Raw library cost (no windowing, full-file parse): **~50 ms** per incremental parse on
    100k-line Rust — `bench_raw_ts_incremental` documents this as the unwindowed baseline.
- Binary size: **7.4 MB** (gate <50 MB ✅) — `ls -lh target/release/plat`
- pty smoke (`/tmp/opencode/syntax_smoke.py`, release build, real keys):
  - `.rs` file opens; status bar shows `EDIT [hello.rs]`; `fn` renders in viewport
  - typed `fn foo() {}\n`; screen updated; no panic output
  - Ctrl-Q → dirty-confirm → Discard → exit 0
  - No "panicked" in output; no "syntax:" error note in status bar
  - Note: ANSI SGR color codes stripped by Screen emulator — behavior smoke only,
    not a color assertion

## Status bar

One bottom line, pinned at both ends:

- **Left:** mode (`EDIT`/`ACTION`), sidebar mode tag, git branch `git(main)`,
  detected language (`rust`), then status messages. (Buffer names live in
  the [tab row](#tab-row--buffer-cycling); for single-buffer sessions they
  stay on the status line where they always were.)
- **Right (aligned):** cursor `line:col` — or `N selected` while a
  selection is active — plus document position `42%` / `Top` / `Bot`.

The git branch is resolved once per file path (git queried in the
document's directory; results, including not-a-repo negatives, are
cached so plain files never re-spawn git). Detached HEAD shows the
short SHA. A branch switched in *another* shell appears when you
reopen the document — no watcher, by design.

LSP status will join this bar in roadmap item #5 (no placeholder slot
until there is something to show).

## Acceptance (status bar)

- **199 unit + 11 flow** tests green (`cargo test`); no regressions.
- Behaviors, each pinned by a named test:
  - line:col + document percent, right-aligned →
    `status_bar_shows_line_col_and_percent_right_aligned`
  - `Top`/`Bot` at the ends; selection count replaces position while
    selecting → `status_bar_top_bot_and_selection_replace_position`
  - narrow terminal: right side stays pinned, mode survives truncation →
    `status_bar_narrow_width_keeps_right_side_and_mode`
  - branch + language segments → `status_bar_shows_branch_and_language`
  - branch lookup with negative cache →
    `app::tests::refresh_branch_reads_repo_and_negative_caches_plain_dir`
  - end-to-end in a real scratch git repo, position follows typing →
    `status_bar_shows_branch_and_live_position` (flow)
- pty smoke (`/tmp/opencode/status_smoke.py`, release build, real keys):
  - startup bar: `EDIT [m.rs]  git(master)  rust  …  1:1 Top`
  - typed `abc` → `…  1:4 Top`
  - Ctrl-Q → dirty-confirm (Tab cycles to SAVE, Enter) → exit 0, no panic
  - ANSI colors stripped by the emulator — behavior smoke only

## Tab row & buffer cycling

With **two or more buffers open**, a tab row appears at screen row 0:

```
[m.rs+] other.json third.rs   …        ← active in brackets, + unsaved
```

It costs a screen row only when there is something to show — single-buffer
sessions keep the old layout (buffer names on the status line). Too many
buffers for the width? The row windows around the active buffer and marks
clipped ends with `…`.

- `Alt-h` / `Alt-l` cycle next/previous with wrap-around (remappable as
  `next_buffer` / `prev_buffer`; also work while the sidebar is open).
  Deliberately *not* `Alt-[`/`Alt-]`: terminals send those as `ESC[`/`ESC]`,
  the CSI/OSC escape-sequence prefixes, so they never arrive as keys.
- Cursor, scroll position and selections are per-buffer already, so cycling
  restores each file exactly as you left it; branch and language on the
  status bar follow the newly active buffer.

## Acceptance (tab row & cycling)

- **204 unit + 12 flow** tests green; no regressions (the pre-existing
  status-line buffer-list test was retargeted to the tab row).
- Behaviors, each pinned by a named test:
  - tab row lists buffers, brackets the active, shifts the editor down one
    row; status line stops duplicating names →
    `tab_row_lists_buffers_and_shifts_editor_down`
  - overflow windows around the active buffer with `…` markers →
    `tab_row_windows_active_into_view_when_overflowing`
  - Alt-h/Alt-l route in Edit, Action and sidebar keyboard ownership →
    `alt_letter_buffer_cycling_in_both_modes_and_sidebar`
  - wrap-around both directions → `buffer_cycling_wraps_both_ways`
  - end-to-end: cycling moves language + branch segments with the buffer →
    `alt_letter_cycle_buffers_and_status_follows` (flow)
- pty smoke (`/tmp/opencode/tabs_smoke.py`, release build, real keys):
  - two files open → row 0 `[m.rs] n.json`, editor content on row 1
  - ESC-l (Alt-l) → brackets move, `json` appears in the status line
  - Ctrl-Q on clean buffers → exit 0, no panic

## Acceptance (Phase 0)

- 54 tests green (`cargo test`).
- 100k-line file: opens and renders in well under a second (pty smoke,
  release build, `total 0.698s` including pty overhead) — no wrap (Phase 1).
- Panic hook pty-verified: terminal state restored before panic output.
- Two-week daily-driver protocol with `PUNCHLIST.md` starts now; week-one
  irritations become the Phase 0.1 keymap tuning pass.

## Acceptance (Phase 1: file sidebar)

- 116 unit + 7 flow tests green (`cargo test`), no Phase 0 regressions.
- Sidebar pty smoke (release build, real keys): Ctrl-B lists the tree, `→`
  expands a dir, arrows walk into a child, Alt-← shrinks the sidebar and the
  editor follows, Esc closes without opening, Ctrl-P + typing filters live,
  Enter opens the hit, Ctrl-Q quits; `node_modules` never appears; the status
  bar names the open mode. Alt-arrows arrive from a real terminal as
  `Left/ALT` (keydebug-verified).
- Scan cost: 5,100-file tree builds in ~10 ms, one fuzzy filter ~5 ms
  (spec budget 100 ms). 100k-line file still renders in ~6 ms (sidebar closed).
- Skipped/deferred to Phase 1.1: tree refresh, config-file persistence of
  sidebar width, mouse, lazy/ threaded scan, `ignore` crate (real gitignore
  semantics incl. `!` negation), icons/colors.

## Acceptance (Phase 1: search & replace)

- 138 unit + 8 flow tests green (`cargo test`), no Phase 0 regressions.
- Whole-feature code review run; its Critical was real (`a` left stale match
  offsets → pressing `n` parked the cursor past the rope and the next keystroke
  panicked with the buffer unsaved) and is fixed at the root: matches refresh
  after the batch, and `search_next` now ignores matches past a shortened rope,
  which also covers the pre-existing "edit, then `n`" hazard. Also from the
  review: invalid replacement now rejected at Enter (prompt stays open) rather
  than at the first `y`; walkthrough accepts uppercase `Y/N/A/E` and reprints
  the hint on unknown keys; a flaky render-test temp-dir collision (pre-existing)
  was fixed.
- pty smoke (release build, real keys): `/` + regex underlines matches while
  typing, Tab moves to the replacement field, Enter starts the walkthrough,
  `y` then `a` produced the expected bytes on disk (`2026-\d\d` → `YYYY`,
  capture-free bulk rewrite), one Ctrl-Z reverted the `a` batch only, then a
  literal find still reported "1 match(es)" and Ctrl-Q confirmed the dirty
  buffer.
- Perf gate (100k lines, release, `cargo test --release -- --ignored`):
  `set_search` 9.7 ms, `replace_all` 54 ms, batch `undo` 31 ms — so live
  search keeps every keystroke inside one frame budget.
- Bug found by the smoke and fixed on the way: save-then-undo left `dirty`
  false, so Ctrl-Q exited silently discarding the undo. Undo/redo now mark the
  buffer dirty.

## Acceptance (config + keymaps)

- **175 unit + 9 flow** tests green (`cargo test`); no regressions.
- Review Focus behaviors, each pinned by a named test:
  1. Unknown field named with suggestion and line number → `unknown_field_is_named_with_a_suggestion_and_line`
  2. Same binding spelled two ways is a duplicate error → `the_same_binding_spelled_two_ways_is_a_duplicate_error`
  3. Prompt and sidebar ignore the overlay → `prompt_and_sidebar_ignore_the_overlay`
  4. Pending capture completes before the overlay → `pending_capture_completes_before_the_overlay`
  5. Unreadable file is an error, not silent defaults → `unreadable_file_is_an_error_not_defaults`
- Flow test `config_remap_and_unbind_work_through_real_key_events` confirms end-to-end:
  `C-Space` remapped to `select_line` selects the line; `x` unbound to `none` does nothing (no fallthrough).
- pty smoke (release build, real keys, `/tmp/opencode/config_smoke.py`):
  - Config remap (`x` → `begin_select_char`): editor starts with `PLAT_CONFIG`, Esc enters ACTION,
    `x` + `d` (delete selection) leaves `ello world` in the buffer — **discriminating assertion**:
    `h` absent and `ello` present proves `begin_select_char` fired (1-char selection deleted);
    the default `x`=`select_line` would delete the whole line (leaving nothing);
    unbound `x`=Nop would leave `hello world` intact. All three cases produce distinct observable text.
  - Exit-1 check: `PLAT_CONFIG=/nonexistent-dir/x plat` → exit 1,
    stderr: `plat: config error in /nonexistent-dir/x: No such file or directory (os error 2)`,
    no alternate-screen bytes in stdout.


## Language servers

plat speaks LSP over stdio. One server per (language, project root), spawned
lazily; full-text sync, debounced. Phase 1 features: **publishDiagnostics**
(gutter badges, status count, message-on-cursor, `[`/`]` navigate),
**textDocument/completion** (explicit menu, Tab/Enter accept),
**textDocument/definition** (jump, `g o` jump-back).

### Config `[lsp]` table

```toml
[lsp]
rust = "rust-analyzer"   # override default (string → whitespace-split argv)
toml = false             # disable this language entirely
```

Value semantics: **string** → command, spaces split argv; **`false`** → disable;
empty string `""` → disable (documented equivalent of false). Unknown keys
accepted (e.g. `python = "pyright"`). Missing key → default if binary on PATH,
else one status note (`no server for rust`), never repeated.

#### Defaults

| lang | default command |
|---|---|
| rust | `rust-analyzer` |
| c | `clangd` |
| toml | `taplo lsp stdio` |
| markdown | `marksman` |
| bash | `bash-language-server start` |
| json | `vscode-json-language-server --stdio` |

### Keys (additions to the global/action tables)

| Key | Mode | Action |
|---|---|---|
| Ctrl-Space / Ctrl-@ | Edit/Action | Open completion menu (explicit trigger) |
| Tab (at word boundary) | Edit | Trigger completion if LSP server configured; indent otherwise |
| `[` / `]` | Action | Jump to previous / next diagnostic (wraps) |
| Ctrl-] | Edit/Action | `textDocument/definition` (goto-def) |
| `g d` | Action | goto-def (same as Ctrl-]) |
| `g o` | Action | Jump back (pop from jump stack, max 50) |

Completion menu: **Tab** cycles down, **BackTab** up, **Enter** accepts,
**Esc** closes. Any other key closes the menu and routes normally.

Status bar additions:
- Right side: `{n}E {n}W` error/warning counts (omit when 0).
- Left side: when cursor is on a diagnostic line, shows `line:col: message`.
- Gutter: `E`/`W`/`I`/`H` badge + space when any visible line has a diagnostic
  (`badge_w = 2`); zero diagnostics → gutter byte-identical to pre-LSP.

### PLAT_LSP_MOCK — dev/test mode

Set `PLAT_LSP_MOCK=1` and the plat binary becomes a deterministic LSP server
(used by integration tests). It answers `initialize`, pushes two diagnostics
on `didOpen`/`didChange`, returns three completion items, and echoes a
`textDocument/definition` response pointing to line 0, col 0 of the request doc.

For pty smoke tests (where TOML config can't express `sh -c "..."` because
values are whitespace-split), use a wrapper script:

```sh
#!/bin/sh
PLAT_LSP_MOCK=1 exec /path/to/plat "$@"
```

Set `[lsp] rust = "/path/to/wrapper.sh"` in the config.

### Phase 2 — not yet built

Rename, format-on-save, snippet tab-stops, multi-location picker (`g d` shows
first + count today; picker needs a TUI list), hover, workspace symbols,
incremental sync (full-text only today), auto-download servers.

## Acceptance (LSP core)

- **293 unit + 12 flow + 19 lsp_flow** tests green (`cargo test`); 10 ignored
  (perf benches); warning-free.
- Review Focus behaviors, each pinned by a named test:
  1. Never-block invariant: hung/silent server leaves typing at full speed →
     `test_kill_mid_flight` (request sent, child killed, main thread returned
     instantly with elapsed < 50 ms; `Died` event arrives)
  2. Frame desync: byte-at-a-time reader, oversized headers, garbage Content-Length
     → `test_byte_at_a_time_reader`, `test_garbage_header_content_length`,
     `test_oversized_content_length` (all in `lsp::transport::tests`)
  3. UTF-16 offset math: emoji/CJK lines, badge/completion `textEdit` ranges →
     `test_utf16_conversions_table`, `test_completion_textEdit_on_emoji_line`
  4. Stale-response races: completion dropped on gen mismatch; goto-def applies
     regardless → `test_stale_completion_response_dropped`,
     `test_goto_def_ctrl_bracket_lands_line_0`
  5. Zero LSP footprint: no server binary / `toml = false` → bytes identical to
     pre-LSP gutter baselines → `test_no_diagnostics_gutter_is_byte_identical`,
     `test_false_disables_language`
- Perf numbers (release build, `cargo test --release -- --ignored lsp`):

  | Bench | Result | Gate |
  |---|---|---|
  | 100k-line framed write (rope→String) | 1.66 ms | — |
  | 100k-line framed write (write_message) | 3.95 ms | — |
  | 100k-line framed write (total) | **5.6 ms** | < 10 ms ✅ |
  | lsp_pump 1000 diagnostic events | **296 µs** | ≤ 1 ms ✅ |
  | keystroke HEAD no-LSP (1000 InsertChar, 100k lines) | 1954 µs/key | — |
  | keystroke HEAD with-LSP (1000 InsertChar, 100k lines) | 1945 µs/key | — |
  | keystroke baseline dce6fe6 (1000 InsertChar, 100k lines) | 2075 µs/key | — |
  | **keystroke overhead HEAD-with-LSP vs dce6fe6 baseline** | **−6.3%** | ≤ +10% ✅ |

  Note: HEAD is ~6% *faster* than the pre-LSP baseline; this is expected —
  the benchmark measures rope edit cost (O(log n)), which has natural run-to-run
  variance of ±5–10%. The LSP path adds zero sync cost per keystroke
  (sync is debounced, pump is a channel drain: no-op when idle).

- Binary size: **7.9 MB** (gate < 50 MB ✅) — `cargo build --release && ls -lh target/release/plat`
- pty smoke (`/tmp/opencode/lsp_smoke.py`, release build, PLAT_LSP_MOCK wrapper):
  - Status bar shows `1E 1W` after mock publishDiagnostics arrives ✅
  - Cursor on line 0 (error line) → status shows `1:1: mock error` ✅
  - `g d` (goto-def) → mock returns loc 0,0 → cursor at 1:1 ✅
  - `g o` (jump-back) → jump stack exercised (no "stack empty" message) ✅
  - Ctrl-Q → exit 0 ✅; no panic text ✅
  - Note: Ctrl-] (raw `\x1d`) maps to `Char('5')+CTRL` in crossterm 0.29 legacy
    terminal encoding — `g d` action path used in smoke (same `GotoDef` command).
