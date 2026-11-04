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

