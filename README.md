# plat — platypus editor

A terminal text editor where **edit mode is home**: typing always goes into
the buffer; **Esc** enters a slim action mode for moving, selecting, acting
on ranges, and searching. Selection-first — no vim grammar, arrow movement.

> **Full user reference — keybindings, commands, configuration, themes —
> lives in [`docs/REFERENCE.md`](docs/REFERENCE.md).** The sections below
> grew feature-by-feature (with per-feature acceptance notes); when they
> drift, `docs/REFERENCE.md` is the source of truth.

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
| Ctrl-N | new buffer (scratch `[No name]` or from missing CLI path) |
| Ctrl-Z / Ctrl-Y | undo / redo |
| Ctrl-F | find |
| Ctrl-B / Ctrl-P | file sidebar: directory tree / fuzzy picker |
| Alt-F | **global search** (`rg`) in the sidebar — see "Global search" |
| Ctrl-Shift-F | global search (kitty-protocol terminals only; legacy sends the Ctrl-F byte) |
| Ctrl-Q | **quit** (dirty → confirm) |
| Ctrl-C / Ctrl-X / Ctrl-V | copy / cut / paste — CUA, works in every mode (copy needs a selection) |
| Ctrl-W / Ctrl-Shift-W | jump to next / previous **word** (close buffer moved to the command palette) |
| Ctrl-< / Ctrl-> | go to **buffer start / end** (kitty-protocol spelling; see note below) |
| Ctrl-. | **code actions** — quickfixes/source actions from the language server |
| Alt-h / Alt-l | previous / next buffer (wraps; works with the sidebar open) |
| Ctrl-O | **suspend to shell** (run lazygit etc., then `fg`) |
| Ctrl-\ | toggle embedded terminal pane |

> Ruling 2026-11-09 (#9): `Ctrl-.` belongs to **code actions**; buffer bounds
> moved to `Ctrl-<`/`Ctrl->`. kitty-protocol terminals deliver two spellings of
> those chords — the raw `</>` codepoints with Ctrl (ESC[60;5u), or the
> unshifted `,`/`.` base with Ctrl+Shift (ESC[44;6u) — pty-probed; plat binds
> all of them, and plain `Ctrl-,` still means buffer start on kitty. Legacy
> terminals drop the Shift bit on control bytes: `Ctrl-,`/`Ctrl-.` arrive as
> the `Ctrl-4`/`Ctrl-6` control bytes (0x1c/0x1e — crossterm reports them that
> way, pty-probed) and there stay buffer start/end — **except** `Ctrl-4`+CTRL,
> which since 2026-11-06 belongs to the embedded-terminal toggle (a legacy
> terminal cannot tell `Ctrl-\` from `Ctrl-,`; the toggle wins). On legacy
> terminals code actions are `:codeaction`/palette only, since `Ctrl-.` there
> *is* the BufferEnd byte.
> `Ctrl-Shift-W` and `Ctrl-Shift-F` need a kitty-protocol terminal (kitty, foot,
> WezTerm, Ghostty): legacy terminals send the same byte as plain `Ctrl-W`/`Ctrl-F`.
> `CloseBuffer` has no default chord anymore —
> close with `:q` (same path) or the command palette (`Ctrl-T`), or bind your
> own via `close_buffer`.
>
> **Kitty keyboard protocol**: plat now pushes the `DISAMBIGUATE_ESCAPE_CODES`
> flag on startup (and pops it on exit/suspend) — on supporting terminals
> (Ghostty, kitty, foot, WezTerm) this splits the legacy control-byte aliases:
> `Ctrl-,` and `Ctrl-\` stop colliding at 0x1c, and Shift-Ctrl chords
> (`Ctrl-Shift-F`, `Ctrl-Shift-W`) arrive distinct. Non-supporting terminals
> silently ignore the sequences and keep the legacy spellings.

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
| `:` | commands: `w [path]`, `q` (close buffer — also the close path now that `Ctrl-W` jumps words), `wq`, `e path`, `cd [dir]` (`~` expands; an open tree/picker re-roots), `format` |

Prompts own the keyboard and live on the bottom **echo line** — the terminal
caret sits right at the input position there, like Helix's command line.
Enter commit · Esc cancel · Up/Down history ·
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
| Tab | toggle expand/collapse of the selected dir |
| Enter | open the selected file (sidebar closes); on a dir, toggle it; on the `..` row, make the parent the new root |
| Alt+Enter | make the selected dir the new root (also works on `..`) |
| type | filter live in **both** modes — the picker ranks matches, the tree keeps its shape (matches + their ancestor dirs); hit letters are underlined |
| Backspace | undo one filter character |
| Esc | first press clears the filter; second press closes the sidebar |
| Alt-← / Alt-→ | shrink / grow the sidebar (10–50 cols, kept for the session) |
| Ctrl-R | rescan the directory tree (expansions and an active filter survive) |
| Ctrl-F | open find while sidebar is open |

The tree scan is **lazy** (#12): opening lists only the root level, and the
first time you expand a directory just that level is scanned (cached —
re-expanding never rescans; `Ctrl-R` is the rescan, and it keeps expansions,
filters and the cursor). Huge/unignored roots and network mounts therefore
cost nothing you don't browse; the status bar reports unreadable dirs. Row
order stays identical to a full DFS scan (dirs first, alphabetical), so
filters keep matches plus their ancestor rows as before — but note the tree
filter now only sees **loaded** rows; for whole-tree lookup use **Ctrl-P**
(the picker takes one complete walk by design, since ranking needs every
path).

Ctrl-S, Ctrl-Q, Ctrl-W, Ctrl-O, Ctrl-Z/Y still work with the sidebar open.
`:e path` still opens a file by name.

The tree draws real connectors (`├─`/`└─` with `│` guides) instead of `+`/`-`
markers, dirs end with a trailing `/`, and dir names carry the `sidebar_dir`
theme role (built-in: bold). Every row is prefixed with a Nerd Font kind icon
(folder for dirs, seti-style glyphs for ~40 known file kinds, generic page
otherwise) tinted per kind — VS Code-explorer style; the colors are built in,
not theme roles. Set `icons = false` under `[sidebar]` to drop them on
terminals without a Nerd Font. Absolute trees (always, in practice) lead with a
virtual `..` row: Enter on it — or Alt+Enter — re-roots one level up;
Alt+Enter on any dir row roots the tree there. Re-rooting clears any filter and
moves the editor's cwd, exactly like `:cd` (running LSP servers keep the old
root until the file reloads). The picker remembers its filter text across
close/reopen; the tree filter starts fresh each open, but the tree itself
remembers: re-opening (same root) restores your expansions and cursor
position — re-rooting starts a fresh tree.
`:cd [dir]` moves plat's working directory; an already-open tree/picker re-roots to the new cwd, and the status-bar git branch refreshes. LSP servers keep the root they were spawned with.

The scan skips `.git`, `node_modules`, `target`, `__pycache__` and `.egg-info`,
plus the rules in a root `.gitignore` (`*.ext`, `dir/`, plain names; no `!`
negation yet). Unreadable directories are skipped and reported in the status bar.

## Recently opened files (#37)

plat keeps a persistent most-recently-opened list: every file you open —
CLI argument, `:e path`, picker/tree click, goto-new-buffer — and every
`save_as`/`:w newpath` target is recorded (canonical path, newest first,
deduped, capped at 25) in `$XDG_DATA_HOME/platypus/recent.json`
(`~/.local/share/platypus/recent.json` by default), written through on each
change. Entries whose file no longer exists are dropped at load — the list
heals itself.

Open the picker with `:recent` (colon) or palette `recent_files`
(no default chord): same popup as the command palette — live subsequence
filter, Up/Down/Enter/Esc, click a row; paths show cwd-relative, a vanished
path reads `gone`. **Starting `plat` with no file arguments and a non-empty
history opens the picker instead of a blank scratch buffer** (Esc falls back
to the scratch you know).

## Global search

**Alt-F** opens a third sidebar mode: workspace search via
[ripgrep](https://github.com/BurntSushi/ripgrep) (`rg` must be on PATH). It is
completely independent of the tree browser — rg owns all scanning and
`.gitignore` logic; the sidebar only displays results and jumps.
`Ctrl-Shift-F` also works on kitty-protocol terminals (legacy terminals send
the plain Ctrl-F byte).

| Key | Action |
|---|---|
| type | the query — each change re-runs rg (150 ms debounce); results stream in live |
| ↑ / ↓, PgUp/PgDn, Home/End | move through hits |
| Enter / click | open the hit's file and place the caret at its line:column (closes the sidebar) |
| Esc | first press clears the query; second press closes (killing any running rg) |
| Ctrl-R | re-run the current query |

Rows read `file:line matched text`; the status bar shows `[search N]`, with
`N+` once the 1000-hit cap truncates and `…` while rg is still running. The
last query is remembered until the next session restart. Search scope is the
sidebar root (cwd, or wherever the tree/picker re-rooted to). Smart-case:
lowercase queries ignore case, any uppercase is literal.

## Format on save

`format_on_save = true` in `[editor]` makes `C-s` / `:w` ask the buffer's
language server (`textDocument/formatting`) to format the file. The save
itself is never blocked: the file is written first, and when the server
answers, the returned edits are applied as **one undo batch** (a single
Ctrl-Z reverts the whole format) and the file is rewritten — the status bar
shows `formatted`. No language server (or one that doesn't format, or a
`null` answer) → plain save, no noise. If you keep typing while the request
is in flight, the stale edits are dropped rather than spliced at wrong
offsets. `:wq` and close-confirm saves stay synchronous: quitting never
waits on a server. plat ships no formatters — the language server does the
work (rust-analyzer, gopls, pylsp, ...).

Manual trigger, no save: `:format` or palette `format_document` (no default
chord, CloseBuffer precedent) formats the active buffer WITHOUT touching the
disk — the edits land as one undo batch and the buffer stays dirty, so you
decide when the formatted text reaches the file. If the server answers with
nothing, `:format` says so: `nothing to format (no edits from server)`.
Python note: plat's default server is `pylsp`, whose built-in formatter needs
`yapf` + `whatthepatch` importable in pylsp's own Python env — without them
pylsp still advertises formatting but returns null (that status is the hint).

## Acceptance (format on save)

- [ ] `[editor] format_on_save = true`; C-s on a server-backed file: disk
      file gains the server's formatting, status shows `formatted`.
- [ ] One Ctrl-Z undoes the entire format.
- [ ] No server / null response / server without formatting: plain save.
- [ ] Typing while the request is in flight → stale edits dropped.
- [ ] Default off; `:wq`/close-save never wait on a server.
- [ ] `:format` / palette `format_document`: buffer formats, disk untouched,
      buffer stays dirty; one Ctrl-Z reverts; no server → status hint, no crash.

## Code actions (#9)

`Ctrl-.` (or `:codeaction`, palette `code_action`) asks the language server
for quickfixes and source actions covering the current selection (or the caret
when there is none), sending the diagnostics the server last published for
this buffer as context — so "fix this import" style fixes appear where the
diagnostics are. The server decides the menu; plat only renders titles.

- Tab / Shift-Tab (or arrows) move, Enter applies, Esc closes — same keyboard
  contract as the completion dropdown.
- Applying edits both `changes` and `documentChanges` WorkspaceEdit shapes;
  each affected document gets ONE undo batch (one Ctrl-Z per file). Files not
  open in plat are read from disk into a new buffer, left dirty, and say so in
  the status line — you review, then `:w`.
- Command-only actions (`action.command`) and file create/rename/delete ops
  inside an edit are reported with a status note and skipped — the plumbing
  lands with the rename work order.
- Typing while the request is in flight → the stale response is dropped.

### Acceptance (code actions)

- [ ] `Ctrl-.` on a line with a diagnostic: picker lists server actions;
      Enter applies them; one Ctrl-Z undoes the whole action.
- [ ] No actions at the caret: status says `no code actions here`.
- [ ] Multi-file edit opens the other file's buffer dirty, one undo per file.
- [ ] Command-only action: status note, nothing applied.
- [ ] `Ctrl-<` / `Ctrl->` jump to buffer start/end (kitty-protocol terminals).

## Markdown preview

`:preview` (colon command) or the command palette renders the current
markdown buffer through [glow](https://github.com/charmbracelet/glow)
(`glow` must be on PATH) into a plain-text buffer: headings, lists,
emphasis — all rendered by glow, with its colors (256-color + bold) carried
into the buffer as styles. Works on
`.md`/`.markdown` files and on unsaved scratch buffers; anything else gets
a status hint instead.

- The preview is a normal buffer: scroll/search/select freely; its path
  stays `None`, so it can never save over the source (use `:w newfile.md`
  deliberately if you want it on disk). Editing it drops the captured
  colors (their coords no longer match the render); `:preview` from the
  source again refreshes both text and colors.
- `:preview` again **on the source file** replaces the same preview buffer
  in place (edit → re-preview is a refresh, not buffer pile-up).
- `:preview` on a preview buffer is refused (`already a preview`).
- Wrapped at the current editor width; resize + re-preview to rewrap.
- External program, not built in: glow stays the markdown renderer — plat
  just pipes stdin and strips SGR (a ~100 ms blocking spawn; if huge docs
  ever lag the loop, the search module's thread+mpsc pattern is the upgrade
  path).

## Configuration

plat reads `~/.config/platypus/config.toml` (or `$XDG_CONFIG_HOME/platypus/config.toml`
if set, or the explicit `PLAT_CONFIG` env var). A missing XDG/default path silently
uses built-in defaults. `PLAT_CONFIG` set to a missing file exits 1 with an error.

```toml
[editor]
line_numbers = true          # show line-number gutter (default true)
indent_width = 2             # Tab inserts this many spaces (1–8, default 2)
key_debug = false            # echo raw key events into the status bar
terminal_height = 10         # embedded terminal pane height in rows (default 10, clamp 3..=(rows-2)/2)
git_gutter = true            # git change markers in the gutter for repo files (default true)
format_on_save = false       # ask the language server to format after C-s / :w (default false)

[sidebar]
width = 32                   # initial sidebar width in columns (10–50)
ignore = ["dist", "*.min.js"]  # extra patterns to skip during scan
icons = true                 # Nerd Font file-type icons (false for plain fonts)

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
| `refresh_syntax` | — (palette only) | reparse current buffer's tree-sitter state |
| `restart_lsp` | — (palette only) | restart the current file's language server |
| `quit` | `Ctrl-Q` | quit (dirty → confirm) |
| `close_buffer` | `:q` / command palette `Ctrl-T` | close buffer (dirty → confirm; last buffer → quit) |
| `format_document` | `:format` / command palette `Ctrl-T` | format buffer via LSP (buffer-only; disk untouched) |
| `code_action` | `Ctrl-.` / `:codeaction` / palette | code actions from the language server (`Ctrl-.` legacy-terminals: `:codeaction`/palette) |
| `next_buffer` | `Alt-l` | cycle to the next buffer (wraps); `Alt-]` is a best-effort alias |
| `prev_buffer` | `Alt-h` | cycle to the previous buffer (wraps) |
| `suspend` | `Ctrl-O` | suspend to shell |
| `open_sidebar_tree` | `Ctrl-B` | open directory tree sidebar |
| `open_sidebar_picker` | `Ctrl-P` | open fuzzy picker sidebar |
| `global_search` | `Alt-F` | global workspace search via `rg` (`Ctrl-Shift-F` on kitty-protocol terminals) |
| `resize_sidebar.grow` | `Alt-→` | widen sidebar by 2 cols |
| `resize_sidebar.shrink` | `Alt-←` | narrow sidebar by 2 cols |
| `toggle_terminal` | `Ctrl-\` | toggle terminal pane visibility |
| `resize_terminal.grow` | `Alt-Up` | increase terminal pane height |
| `resize_terminal.shrink` | `Alt-Down` | decrease terminal pane height |
| `enter_action_mode` | `Esc` (in Edit) | enter action mode |
| `enter_edit_mode` | `Esc` (in Action) | return to edit mode |
| `indent` | `Tab` | insert indent |
| `insert_newline` | `Enter` | insert newline |
| `backspace` | `Backspace` | delete char before cursor |
| `delete` | `Delete` | delete char after cursor |
| `move.<dir>` | arrows, `w`/`b` (Action), `Ctrl-W`/`Ctrl-Shift-W` | move cursor (`left` `right` `up` `down` `pageup` `pagedown` `word_next` `word_prev` `line_start` `line_end` `buffer_start` `buffer_end` — the buffer pair also binds `Ctrl-<` / `Ctrl->` (plus the legacy `Ctrl-,` / `Ctrl-.` spellings) |
| `move.<dir>.extend` | `Shift`+arrows | extend selection |

## Syntax highlighting

plat uses tree-sitter for syntax highlighting. Nine languages are supported out of the box:

| Language | Extensions |
|----------|------------|
| Rust | `.rs` |
| TOML | `.toml` |
| Markdown | `.md` |
| JSON | `.json` |
| C / C headers | `.c`, `.h` |
| Bash | `.sh`, `.bash` |
| Python | `.py`, `.pyi` |
| Go | `.go` |
| Lua | `.lua` |

Grammar queries also emit extra scope names (`variable`, `operator`, `conditional`,
`repeat`, `boolean`, `parameter`, …); unmapped scopes render with the default
foreground and are themeable via `[theme.syntax]` scope entries.

Highlighting is on by default and can be disabled via config:

```toml
[editor]
syntax = false   # disable syntax highlighting (default: true)
```

Colors are fully themeable — see [themes in `docs/REFERENCE.md`](docs/REFERENCE.md).

## Acceptance (syntax highlighting)

- **194 unit + 10 flow** tests green (`cargo test`); no regressions.
- Review Focus behaviors, each pinned by a named test:
  1. Incremental parse matches a fresh rebuild → `incremental_matches_fresh_rebuild`
  2. Spans use char columns (not byte offsets) on multibyte lines → `spans_use_char_cols_on_multibyte_lines`
  3. Edit accumulates only when syntax is present; plain text has no styles; config off prevents build →
     `edit_accumulates_only_when_syntax_present`, `plain_text_document_has_no_syntax_styles`,
     `syntax_disabled_via_config_prevents_build`
  4. All nine grammars construct → `all_nine_grammars_construct`
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
- Binary size: **8.8 MB** (gate <50 MB ✅) — `ls -lh target/release/plat`
- pty smoke (`/tmp/opencode/syntax_smoke.py`, release build, real keys):
  - `.rs` file opens; status bar shows `EDIT [hello.rs]`; `fn` renders in viewport
  - typed `fn foo() {}\n`; screen updated; no panic output
  - Ctrl-Q → dirty-confirm → Discard → exit 0
  - No "panicked" in output; no "syntax:" error note in status bar
  - Note: ANSI SGR color codes stripped by Screen emulator — behavior smoke only,
    not a color assertion

## Status bar

Two bottom lines (Helix-style): the status bar sits on the second-to-last
row, the **echo line** is the last row. The status bar, pinned at both ends:

- **Left:** mode (`EDIT`/`ACTION`), sidebar mode tag, git branch `git(main)`,
  detected language (`rust`), file format (`LF utf-8`). (Buffer names live in
  the [tab row](#tab-row--buffer-cycling); for single-buffer sessions they
  stay on the status line where they always were.)
- **Right (aligned):** cursor `line:col` — or `N selected` while a
  selection is active — plus document position `42%` / `Top` / `Bot` and
  diagnostic counts (`2E 1W`).

The **echo line** (last row) carries everything transient: the active prompt
(`find > [pat] | replace > rep`, `:> w`, `save as > name`, confirm choices),
status messages, and the message for the diagnostic under the cursor.
Errors render in the `diag_error` color; the prompt's caret sits at the
input position. Empty row when there is nothing to say.

The git branch is resolved once per file path (git queried in the
document's directory; results, including not-a-repo negatives, are
cached so plain files never re-spawn git). Detached HEAD shows the
short SHA. A branch switched in *another* shell appears when you
reopen the document — no watcher, by design.

### Git gutter (#13)

Documents inside a git repository get a one-column gutter left of the
line numbers: `▍` green = line added since HEAD, `▍` yellow = modified,
`▍` red = a removal boundary (the row the lines vanished before). The
marks come from an asynchronous `git diff HEAD -U0` — the UI never
blocks — and are refreshed when a file opens, after every save, and
after external disk changes; while a buffer is unsaved the marks are
as-of the last save. Files outside a repo (or with `git_gutter = false`
in `[editor]`) show no column at all; the column is reserved per
document as soon as the repo is known, so text never shifts. Deleted
marks at end-of-file have no row to mark and stay invisible.

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

## Echo line (bottom command row)

The bottom row is an echo/command line (Helix-style): the status bar moved
one row up and the editor lost one row. Prompts (find, save-as, `:`,
confirms), status messages and cursor-on-diagnostic messages all render
there; the terminal caret follows the prompt's input position.

## Acceptance (echo line)

- **351 unit + 14 flow + 19 lsp_flow** tests green; every former
  last-row status pin retargeted to the second-to-last row.
- Behaviors, each pinned by a named test:
  - prompts leave the status bar → `prompt_renders_on_echo_line_not_status_bar`
  - save-as / command / confirm prompts on the echo row →
    `save_as_and_command_prompts_render_on_echo_line`,
    `confirm_prompt_renders_on_echo_line`
  - error messages in `diag_error` color, info in `status_bar` color →
    `echo_line_error_messages_use_diag_error_color`
  - real terminal caret on the echo row at the input position →
    `terminal_cursor_sits_on_echo_line_during_prompt` (TestBackend
    `cursor_position`)
  - messages no longer share the status bar → updated
    `status_bar_narrow_width_keeps_right_side_and_mode`
- pty smoke (release build, real keys): `/def` paints terminal row 24;
  unknown `:zz` error repaints row 24; no panic.

## Tab row & buffer cycling

With **two or more buffers open**, a tab row appears at screen row 0, left-aligned:

```
[m.rs+] × other.json third.rs …  ← left-aligned; active bracketed + ×
```

It costs a screen row only when there is something to show — single-buffer
sessions keep the old layout (buffer names on the status line). Too many
buffers for the width? The row windows around the active buffer and marks
clipped ends with `…`. The `×` marks the active tab's close affordance —
clicking it closes that buffer (wired by mouse support, work-order #11).

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
- Scan cost: 18,360-item tree builds in ~18 ms, one filter keystroke ~3 ms,
  cursor move ~0 ms (perf pass 2026-11-05: cached expand-view row indices +
  index-based filtered rows; was 11 ms/keystroke, 0.7 ms/move on 18k items).
  Spec budget 100 ms. 100k-line file still renders in ~6 ms (sidebar closed).
- Unreadable dirs are skipped silently while the TUI owns the terminal —
  nothing may write to stderr mid-session (raw TTY text desyncs ratatui's
  diff and corrupts the display; seen as injected "Permission denied"
  fragments). The status line reports `sidebar: N dir(s) unreadable` instead.
- Tabs are expanded to 8-col stops by the renderer (and control chars in
  filenames/menus map to spaces): raw TAB cells would flush to the TTY, where
  the terminal re-expands them at different columns than the app's cell model
  — every later diff frame then paints at wrong x (the "text overlap/scramble"
  on tab-indented Go, 2026-11-05). Spans, selection, find underlines and the
  caret all map through a per-line char→display table, so highlight colors
  follow tab-expanded text (the "same keyword, different colors" report).
- Skipped/deferred to Phase 1.1: tree refresh, config-file persistence of
  sidebar width, lazy/ threaded scan, `ignore` crate (real gitignore
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


## Themes

plat ships two built-in palettes — `dark` (default) and `light` — and loads
per-user theme files from the **`themes/`** directory next to your config file
(e.g. `~/.config/platypus/themes/<name>.toml`).

### Selecting a theme

```toml
# Alternative A — built-in palette:
[editor]
theme = "dark"   # or "light"

# Alternative B — custom file (~/.config/platypus/themes/my.toml):
[editor]
theme = "my"
```

### Theme file format

```toml
[ui]
# 18 UI role names:
default        = { fg = "white", bg = "black" }  # base text fg/bg for the
                                                # editor area (gutter incl.);
                                                # syntax/selection patch on top
line_no        = "#45475a"                       # plain gutter numbers
                                                # (built-in: inherit `default` fg)
current_line_no = { bold = true, underline = true }  # active line's gutter number
status_mode   = { fg = "black", bg = "white" }  # the mode word ("EDIT"/"ACTION+")
status_bar    = { fg = "white", bg = "blue" }
tab_active    = { bold = true }
tab_inactive  = "dark-gray"
selection     = { bg = "#005f87" }
completion_sel = { bg = "blue" }
menu_bg       = { bg = "dark-gray" }   # dropdown rectangle (completion+actions, #40)
sidebar_sel   = { bg = "blue" }
sidebar_dir   = { bold = true }              # dir names in the sidebar (built-in: bold)
find_match    = { underline = true }
diag_error    = "red"
diag_warn     = "yellow"
diag_info     = "dark-gray"
diag_hint     = "dark-gray"

[syntax]
# Scope names match tree-sitter node types (longest prefix wins):
keyword  = "#ff0000"
function = { fg = "cyan", bold = true }
comment  = { fg = "dark-gray", italic = true }
string   = "green"
```

### Color grammar

| Form | Example | Meaning |
|---|---|---|
| Named | `"red"` | ANSI named color (black white red green yellow blue magenta cyan) |
| Bright | `"bright-red"` | Bright/light variant (bright-red … bright-cyan, bright-white); `light-*` is accepted as an alias for `bright-*` |
| Dark alias | `"dark-cyan"` / `"dark-yellow"` | Maps to xterm Cyan / Yellow (normal, not bright) |
| Gray | `"gray"` / `"grey"` / `"dark-gray"` | Gray tones |
| Reset | `"reset"` | Terminal default |
| Hex | `"#rrggbb"` | 24-bit RGB (e.g. `"#005f87"`) |
| Index | `240` | xterm 256-color index (0–255, as a TOML integer) |

Short form: a bare string or integer sets the foreground color. Use a table
(`{ fg = …, bg = …, bold = …, italic = …, underline = … }`) to set multiple
attributes; omit any you don't want to override.

### Layered precedence

1. **Dark base** — built-in defaults for all roles and syntax scopes.
2. **Theme file** (`themes/<name>.toml`) — overrides on top of the dark base;
   the built-in `light` palette uses a curated override set.
3. **Config overrides** (`[theme.ui]` / `[theme.syntax]` in `config.toml`) —
   highest priority, applied regardless of which theme file is active.

### Error policy

Unknown theme name → falls back to dark, adds a startup note (not fatal).
Unknown role keys, bad color values → noted in the status bar, never a crash.
A theme file that fails to parse → noted, theme falls back to dark base.

## Acceptance (themes)

- **319 unit + 13 flow + 19 lsp_flow** tests green (`cargo test`); warning-free.
- `theme_e2e_file_keyword_selection_and_completion_sel` (flow): real config.toml +
  themes/e2e.toml on disk (`PLAT_CONFIG` isolation, pid-unique dir); asserts
  keyword fg=Red, selection bg=Rgb(0,255,0), completion_sel bg=Magenta — all GREEN.
- pty smoke (`/tmp/opencode/theme_smoke.py`, release build, `COLORTERM=truecolor`):
  - `keyword fg #ff0000` → `38;2;255;0;0` **FOUND** in raw pty stream ✓
    context: `…\x1b[38;2;255;0;0;49mfn\x1b[…`
  - `selection bg #00ff00` → `48;2;0;255;0` **FOUND** after Shift-Right select ✓
    context: `…\x1b[38;2;255;0;0;48;2;0;255;0mf\x1b[…`
- Layered precedence: `file_shadows_builtin`, `longest_prefix_wins_under_overrides`,
  `per_key_merge_inherits_base_style`, `bad_values_warn_not_fatal` (all in `theme.rs`).

## Language servers

plat speaks LSP over stdio. One server per (language, project root), spawned
lazily; full-text sync, debounced. Phase 1 features: **publishDiagnostics**
(gutter badges, status count, message-on-cursor, `[`/`]` navigate),
**textDocument/completion** (explicit menu, Tab/Enter accept; the menu is
client-side filtered against the word before the caret — LSP servers like
gopls/rust-analyzer return their whole scope and expect the client to filter —
and typing while the menu is open re-requests, so items and edit ranges stay
fresh),
**textDocument/definition** (jump, `g o` jump-back).

### Config `[lsp]` table

```toml
[lsp]
rust = "rust-analyzer"   # override default (string → whitespace-split argv)
python = ["ruff", "server"]  # array → exact argv (elements keep spaces intact)
toml = false             # disable this language entirely
```

Value semantics: **string** → command, spaces split argv; **array** → exact
argv (use when an argument itself contains spaces); **`false`** → disable;
empty string `""` or empty array `[]` → disable (documented equivalent of
false). Unknown keys
accepted (e.g. `python = "pyright"`). Missing key → the built-in default is
attempted (spawned when a matching file opens); if its binary is missing,
one status note per language (`no server for rust`), no further attempts
that session. Servers may also explain themselves: `window/showMessage`
text surfaces in the status bar as `lsp: …` (e.g. rust-analyzer's
"Failed to discover workspace" advice when a lone file has no project).

#### Defaults

| lang | default command |
|---|---|
| rust | `rust-analyzer` |
| c | `clangd` |
| toml | `taplo lsp stdio` |
| markdown | `marksman` |
| bash | `bash-language-server start` |
| json | `vscode-json-language-server --stdio` |
| python | `pylsp` |
| go | `gopls` |
| lua | `lua-language-server` |

### Config `[formatting]` table (external formatters)

`[lsp]` picks the server that provides intelligence (completion, diagnostics,
code actions); `[formatting]` overrides **who formats** for a language —
use it when the LSP server has no formatter (pylsp without yapf, pyright) or
you prefer a CLI (ruff's server provides no completion; `ruff format -` does
the formatting). Value semantics are identical to `[lsp]`: **string** →
whitespace-split argv, **array** → exact argv, **`false`**/`""`/`[]` →
disable formatting for that language. This table has no built-in defaults.

```toml
[lsp]
python = "pylsp"           # completion + diagnostics

[formatting]
python = "ruff format -"   # whole buffer via stdin -> stdout
rust = false               # no formatting for rust at all
```

Routing: `:format_document` and `format_on_save` check `[formatting]` first —
if set, the buffer is piped to the command's stdin (worker thread, the UI
never blocks) and stdout replaces the buffer as **one undo step**. Typing
while the formatter runs drops the result (stale-edit guard, same rule as
LSP formatting); a non-zero exit shows `formatter failed: …` and leaves the
buffer untouched. Absent key → `textDocument/formatting` on the LSP server,
exactly as before.

### Keys (additions to the global/action tables)



| Key | Mode | Action |
|---|---|---|
| Ctrl-Space / Ctrl-@ | Edit/Action | Open completion menu (explicit trigger) |
| Tab (at word boundary) | Edit | Trigger completion if LSP server configured; indent otherwise |
| `[` / `]` | Action | Jump to previous / next diagnostic (wraps) |
| Ctrl-] | Edit/Action | `textDocument/definition` (goto-def) |
| `g d` | Action | goto-def (same as Ctrl-]) |
| `g o` | Action | Jump back (pop from jump stack, max 50) |
| Ctrl-G `d` / Ctrl-G `o` | Edit/Action | goto-def / jump back without leaving Edit mode (a mistyped third key just cancels — but swallows that one keypress) |

Jumps never change the mode: `g o` chains straight after `g d` — even when the
definition opened a new buffer — and a Ctrl-] from Edit keeps your typing context.

Completion menu: **Tab** cycles down, **BackTab** up, **Enter** accepts,
**Esc** closes. Any other key closes the menu and routes normally.

Status bar additions:
- Right side: `{n}E {n}W` error/warning counts (omit when 0).
- Left side: when cursor is on a diagnostic line, shows `line:col: message`.
- Gutter: `E`/`W`/`I`/`H` badge + space once the document has ANY diagnostic —
  the 2-column gutter reservation is stable (never jumps when scrolling);
  zero diagnostics → gutter byte-identical to pre-LSP.

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
diagnostics list view, incremental sync (full-text only today), auto-download servers.

## Acceptance (LSP core)

- **299 unit + 12 flow + 19 lsp_flow** tests green (`cargo test`); 10 ignored
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
- Real-server end-to-end (rust-analyzer 1.96.0, tiny crate): status shows `1E` for
  an `mismatched types` error; gutter badge column reserved on all rows with the
  `E` cell on the diagnostic row; detached-file case surfaces the server's own
  `window/showMessage` explanation instead of silence (`window_show_message_becomes_server_message_event`).
- A single dead server emits exactly ONE `Died` (reader EOF + writer EPIPE are
  two reporters of one death) — `one_dead_server_emits_exactly_one_died`.
- pty smoke (`/tmp/opencode/lsp_smoke.py`, release build, PLAT_LSP_MOCK wrapper):
  - Status bar shows `1E 1W` after mock publishDiagnostics arrives ✅
  - Cursor on line 0 (error line) → status shows `1:1: mock error` ✅
  - Ctrl-] (raw `\x1d`, goto-def) → mock returns loc 0,0 → cursor at 1:1 ✅
  - `g o` (jump-back) → restores the pre-jump cursor readout exactly ✅
  - Ctrl-Q → exit 0 ✅; no panic text ✅
  - Note: crossterm 0.29 maps the Ctrl-] byte (`\x1d`) to `Char('5')+CTRL` on
    non-kitty terminals; keymap binds both that and `Char(']')+CTRL` (kitty).

## Command palette

`C-t` opens a centered popup with all 20 available commands, their descriptions, and live key hints.
Type to filter by subsequence match (e.g. `save` narrows to Save / Save As). `↑`/`↓` select;
`Enter` runs the highlighted command through the normal command path; `Esc` closes.

Two refresh commands have no default key (reach them via the palette, or bind one yourself):
`refresh_syntax` drops and reparses the current buffer's tree-sitter state (fixes stale
highlighting after a bulk edit); `restart_lsp` kills the language server for the current
file's language and re-attaches every open buffer of it (fresh `didOpen`, cleared crash/
spawn gates, current cwd as root — the fix when LSP looks wedged or after a `:cd`).
Bind them with e.g. `"\\" = "restart_lsp"` in `[keys]`.

Key hints reflect your `[keys]` remaps — if you rebind `save` to `C-m`, the palette shows `C-m`.
The trigger itself is remappable: `command_palette = "ctrl+t"` in `[keys]`, or `"none"` to unbind.
Save As reuses the existing prompt (not a duplicate widget).

| Key | Action |
|---|---|
| `C-t` (default) | Open command palette |
| Type | Filter commands (subsequence) |
| `↑` / `↓` | Move selection |
| `Enter` | Run selected command |
| `Esc` | Close palette |

## Acceptance (command palette)

- **339 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); warning-free.
- `flow_command_palette_opens_filters_runs_save` (flow): opens a real file, dirties the buffer
  via `EnterEditMode` + `InsertChar('x')`, opens the palette via `CommandPalette`, filters to
  `save`, hits Enter — asserts palette closed and `app.doc().dirty == false` (file on disk) — GREEN.
- pty smoke (`/tmp/opencode/palette_smoke.py`, release build):
  - popup border `┌` **FOUND** in raw pty stream after C-t ✓
    context: `…';30;30;46m┌──────────'…`
  - palette description `save current buffer` **FOUND** after C-t ✓
    context: `…'          save current buffer         C'…`
  - file on disk contains the typed edit after `save` + Enter ✓
    disk: `'xfn main() {}\n'`

## Mouse

Work-order #11. plat captures the mouse while its TUI is up (SGR reporting:
clicks, wheel, drags arrive as events). **Plain drag selects text inside
plat**; **Shift-drag still does the terminal's native selection** (the
usual convention — reach for it when you want the system clipboard).

Every click is hit-tested through the very same geometry `draw()` paints
with (`render::layout` / `tab_line` / `popup_geom` / `sidebar_view` are the
single source of truth), so clicks can never disagree with what you see:

- **Editor click** places the caret on that cell (clamped to line end).
  **Press-drag-release** selects from the press point (the selection clamps
  to the visible view — extend long ranges with Shift+click).
  **Shift-click** also extends the selection from the previous caret;
  modifiers are read from the press event too (terminals differ).
  Ctrl-click multi-caret is deferred to the multiline-cursor polish (#13).
- **A typed char, Backspace, Delete, Indent or paste REPLACES the active
  selection** (undo-friendly single step), like every text editor.
- **Tab row**: click a tab to activate it; click the active tab's `×` to
  close it — a dirty buffer gets the same confirm prompt as `close_buffer`.
- **Sidebar**: clicking a row does exactly what Enter on that row does —
  opens a file, toggles a dir, re-roots on `..`.
- **Border drag**: press the sidebar's right border and drag to resize its
  width live, with the same MIN/MAX clamps as the keyboard resize.
- **Wheel**: three rows per notch — caret (and view) in the editor, row
  cursor in the sidebar, highlighted command in the palette.
- **Palette**: click a row to select and immediately run it; click anywhere
  outside the popup to dismiss it.
- Clicking the editor area dismisses an open sidebar — that's what the click
  is visually pointing at. A modal prompt owns the screen: while one is
  open, clicks do nothing.
- Terminal caveat: some emulators (VTE-family: gnome-terminal and friends)
  swallow Shift+click for their own selection, so plat's Shift+click never
  arrives there. Drag-select and Shift-drag (native copy) both work there —
  that's the escape hatch the `[ui] mouse = false` switch also covers.

### Config `[ui]`

```toml
[ui]
mouse = true   # false leaves mouse reporting to the terminal (no clicks, native drag-select)
```

## Acceptance (mouse)

- **414 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); warning-free.
  Covers: `layout()` mirroring the drawn frame (tab row, sidebar split, gutter
  width from line count, reserved badge column), hit-test tables through
  gutter/badge/scroll offsets and the scrolled sidebar head, tab spans incl.
  the shifted `×` on dirty labels, caret placement + Shift-extend anchors,
  close-confirm path, border-drag resize, wheel navigation in all three
  regions, the `[ui] mouse` config parse, and now drag-selection with
  selection-aware input (typed char, Backspace, Delete, Indent, paste).
- pty smoke (`/tmp/opencode/mouse_smoke.py`, release build, SGR events): 17
  checks all PASS — `?1006h` requested at startup, click caret (status
  `4:8`), wheel (`7:8`), sidebar row click opens the file and shows the tab
  row, tab click switches, `×` closes (dirty prompt verified at unit level),
  border drag shifts editor text 35→43, palette row click runs `open_file`,
  outside click dismisses, and `[ui] mouse = false` requests no mouse modes.
- Selection follow-up (user report "selection is not working", same day):
  four root causes fixed with probes in `app::sel_probe` — press-drag-release
  select (new `render::editor_cell` clamped lookup), modifiers read from the
  PRESS event (terminal-dependent release reporting), typed char /
  Backspace / Delete / Indent / paste now consume the selection
  (`App::take_selection` — single undo step), and the replace-walkthrough's
  current-match selection can no longer linger stale over replaced text
  (`highlight_flow_match` clears it when no match follows). pty repro
  (`/tmp/opencode/sel_repro.py`) proves all gestures on the release build.
- Gate fallout fixed: `test_diag_ux_end_to_end` was a latent test flake
  (~1 in 5 full-suite runs, reproducible at the previous commit) — every
  `lsp_flow` test shared one temp root per process and raced file content
  between threads; each test now gets its own dir. 8/8 clean full-suite
  runs afterwards.

## Embedded terminal pane

**Ctrl-\** toggles a persistent interactive shell in a bottom pane. The shell runs in plat's cwd; `cd`, history, and tab completion survive toggling. Type while the pane is visible to send input to the shell; printable text, arrows, and function keys all route to the shell. Key exceptions (still global even when focused on terminal):

- **Ctrl-\** — toggle pane visibility
- **Ctrl-Q** — quit plat
- **Ctrl-O** — suspend to shell
- **Alt-Up / Alt-Down** — resize pane height (3 to half-screen)
- **PgUp / PgDn** — scroll the pane through its scrollback (2000-line
  history; each press moves one pane-page). The status bar marks the scrolled
  state (`shell: … (scrollback +N)`) and the caret hides while scrolled
  back — any key typed into the shell snaps the view back to live output.
  History is vt100's **native, cell-accurate scrollback ring**: exactly the
  rows that scrolled off the top, **colors and attributes included**; while
  the pane is scrolled, `Ctrl-\` hide/show still preserves the scroll
  position. (Full-screen apps switching to the alternate screen — vim, lazy
  — leave no history rows behind; that's standard terminal behavior.)

`:term` command also toggles the pane.

### Config

```toml
[editor]
terminal_height = 10   # default 10 rows; clamps to 3..=(rows-2)/2
```

Process and scrollback persist while the pane is hidden; hiding is visibility-only. One terminal pane per plat instance. Status bar shows `shell: <cwd>` when visible or `shell exited` if the shell has terminated. Shell receives SIGINT on Ctrl-C (typed into the pane); plat retains Ctrl-Q (quit) and Ctrl-O (suspend) globally.

**Scrollback (shipped):** Scrollback is vt100's **native, cell-accurate ring** (2000 rows; oldest evict as they scroll off the top). **PgUp/PgDn** page the pane through that history (one pane-height per press; vt100 clamps to the available rows); the status bar marks the scrolled state (`shell: … (scrollback +N)`) and the caret hides while scrolled. Any key typed into the shell snaps the view back to live output. History rows keep their true fg/bg colors — the scrolled view is literally `screen.cell()` under a `set_scrollback` offset; hiding/showing the pane preserves the scroll position. Verified end-to-end by `term_scrollback_smoke.py` + `colored_scrollback_smoke.py` (pty) and a screen-reconstruction check (`scr_repro.py`) that diffs the actual pane rows.

## Acceptance (embedded terminal pane)

- **497 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); zero warnings.
- Behaviors:
  - Terminal spawns in plat's cwd, persistent across hide/show (`toggle_terminal`, `:term`, `Ctrl-\`)
  - Alt-Up/Down resize height (clamped 3..=(rows-2)/2); `[editor] terminal_height` config parses and clamps
  - All keys route to shell except: `Ctrl-\` (toggle), `Ctrl-Q` (quit), `Ctrl-O` (suspend), Alt-Up/Down (resize), PgUp/PgDn (scroll the vt100-native history view)
  - Ctrl-C in the pane sends SIGINT to the shell, not plat
  - Legacy terminal (`Ctrl-\ = 0x1c = Ctrl-,`): the toggle wins the byte collision on ALL presses — Ctrl-\ opens the first pane too; legacy `Ctrl-,` as BufferStart is shadowed (kitty terminals unaffected)
  - 2000-line vt100-native scrollback ring (cell-accurate rows with colors, captured as they scroll off the top); evicts oldest on overflow

  - Pty device responses (ESC[6n, ESC[5n, ESC[c, ESC[>c) are shim-answered; pump is nonblocking (`O_NONBLOCK`)
- Pinned behaviors (unit + flow tests):
  - `terminal_pane_reader_fd_is_nonblocking`, `terminal_pane_pump_output_feeds_emulator_without_blocking` — pump never blocks the main loop
  - `key_owner_prompt_beats_terminal` — prompts own the keyboard over terminal
  - `key_owner_legacy_ctrl4_routes_legacy_toggle_when_pane_exists`, `key_owner_legacy_ctrl4_opens_pane_when_none` — legacy terminal Ctrl-\ opens AND hides the pane (byte-collision ruling 2026-11-06)
  - `key_owner_terminal_visible_plain_char`, `key_owner_ctrl_backslash_not_terminal_when_pane_visible` — exception key handling
  - `apply_resize_scroll_clamp_honors_visible_terminal_height` — layout respects both pane height config and screen constraints
  - `toggle_terminal_clamps_spawn_height_to_screen_half` + startup `last_size` seed in `run()` — pane spawns at real screen width and screen-clamped height (fixes configured height collapsing on first Alt-↑/↓)
  - `layout_clamps_oversized_pane_on_window_shrink` — terminal shrinks with window, stays within bounds
  - `terminal_pane_exited_after_shell_exit_command` — shell exit lifecycle
  - `native_scrollback_scrolls_cells_view`, `native_scrollback_preserves_colors`, `pane_view_and_status_report_native_offset` — the vt100 native ring IS the scroll view; scrolled-off cells keep their SGR colors
- Binary size: **9.2 MB** (gate < 50 MB ✅)
- Pty smoke (`/tmp/opencode/terminal_smoke.py`, release build, 40x100 pty — deliberately not 24x80): 7/7 PASS — Ctrl-\ (0x1c) opens the FIRST pane with no prior :term, `tput cols; tput lines` shows real width 100 + height 10, `echo hello-plat-terminal` visible, second Ctrl-\ hides, clean exit 0, no panic

## Acceptance (global search)

- **507 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); zero warnings.
- Behaviors:
  - `Alt-F` opens the search sidebar WITHOUT any tree scan (independent of the tree browser); `Ctrl-Shift-F` on kitty-protocol terminals; palette entry `global_search`; remappable/unbindable by name
  - rg (`--vimgrep -S`) runs under the sidebar root, streamed via a reader thread + mpsc; query change kills+respawns with 150 ms debounce; 1000-hit cap flags truncation (`[search N+]`), `…` while running
  - Enter/click on a hit routes through the SAME `apply_goto_location` primitive as goto-def: opens the buffer, caret at line:col, sidebar closes
  - Esc clears the query first, second Esc closes and kills rg; close/quit always reap the child (`SearchJob::stop`)
  - Kitty keyboard protocol push/pop (`ESC[>1u` / `ESC[<1u`, DISAMBIGUATE_ESCAPE_CODES) at startup, exit, panic hook, and around suspend/resume
- Pinned behaviors (unit tests):
  - `search::tests::parse_*` — vimgrep line parsing incl. paths containing colons (leftmost-numeric-split rule) and 1-based→0-based conversion
  - `search::tests::spawn_drains_real_rg_and_reaps` — real rg streamed, child reaped
  - `search_mode_push_filter_label_and_cap` — streaming push, identity rows under re-filter, `name:line text` label, cap + truncation flag
  - `search_debounce_types_streams_and_enter_jumps` — full flow: typing → debounce (injected clock, no sleep) → streamed hits → Enter → caret lands on the hit's line
  - `global_search_opens_without_tree_and_empty_query_spawns_nothing`, `closing_sidebar_stops_the_job`
  - `global_search_chords` — Alt-F (both modes), C-S-F, sidebar table variant, C-F stays Find, remap name resolves
- Binary size: **9.2 MB** (gate < 50 MB ✅)
- Pty smoke (`/tmp/opencode/search_smoke.py`, release build, 40x100 pty): 8/8 PASS — `ESC[>1u` push at startup, legacy ESC-f opens search, live-typed query shows row label `b.txt:2 <token>`, Enter jumps (search UI gone, token painted), clean exit 0, `ESC[<1u` pop, no panic

## New file (open-or-create, `C-n`)

Ad-hoc feature (user asked "how to create a new file?") — three pieces, all
shipped:

- `plat <path>` on a missing file opens an empty buffer with that path set,
  clean (Helix behaviour). The first save creates it on disk; parent
  directories are auto-created if missing (`fs::create_dir_all` pre-write).
- `new_buffer` command: **C-n** (or palette `new_buffer`) adds a fresh
  `[No name]` scratch buffer with no path yet; switch to it, type, then
  `save_as` (palette) to name it. Mode stays Edit; active buffer becomes the
  new one.
- **Typed char, Backspace, Delete, Indent, paste all REPLACE an active
  selection** — not just insert/delete at cursor. Same semantics plat gained
  for the mouse: a selection now means something (was inert before). Undo is
  one step (selection range + text as one edit via `Document::edit`).

Tests: 4 new + 414 unit + flow suite ✓. Pty smoke: launch `plat missing.rs`,
type content, C-s save prompt, enter a nested path `src/new/deep/file.rs`
→ creates parents, writes file.

## CLI directory argument

Path arguments to `plat` follow these rules:

- **Directory:** opens the directory tree browser (Ctrl-B) rooted there. `plat src` opens the tree at `src/`; `plat .` opens at the current directory. Only the first directory is used as the tree root; additional paths are treated as files.
- **File (exists or missing):** opens it as a buffer. `plat file.rs` opens the file; `plat missing.rs` opens an empty named buffer (ready to save).
- **Mixed args:** `plat src/file.rs src/` opens the file as a buffer and roots the tree at `src/`.

Combined with the sidebar's re-rooting (`Alt-Enter` on a dir, or `Enter` on `..`), navigation flows like `plat .` → browse → re-root → browse without re-launching.

## Acceptance (CLI directory)

- **3 new CLI tests** green: `cli_directory_argument_opens_tree_browser`,
  `cli_mixed_file_and_directory`, plus one existing `cli_missing_path_opens_empty_named_clean_buffer`.
- **416 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); warning-free.
- Behaviors:
  - Directory arg roots the tree and opens the sidebar in tree browser mode
  - File args (existing or missing) open as buffers
  - First directory wins if multiple directories are passed
  - `:cd [dir]` updates `tree_root` and refreshes the sidebar as before
- Binary size: **8.9 MB** (gate < 50 MB ✅)

## Acceptance (markdown preview)

- **519 unit + 14 flow + 19 lsp_flow** tests green (`cargo test`); zero warnings.
- Behaviors:
  - `:preview` / palette `markdown_preview` (remappable name, no default chord by design — CloseBuffer precedent) spawn `glow -s dark -p --width <editor width>`, feed the rope on stdin, convert glow's SGR into buffer style spans (basic/256/RGB colors + bold/dim/italic/underline), drop non-SGR CSIs, collapse blank runs; edited preview buffers drop the stale spans (`dirty` guard)
  - `.md`/`.markdown` files and scratch buffers accepted; anything else → status hint, nothing opens
  - Preview buffer tagged `preview_source`; re-preview from the same file replaces in place; `:preview` on a preview buffer refused; preview path stays `None` (never saves over the source)
  - glow missing → status `markdown preview: ... (is glow installed?)`, no crash
- Pinned behaviors (unit tests):
  - `preview::tests::clean_ansi_*` — SGR + cursor-move stripping, trailing pad trimmed, leading indent preserved
  - `preview::tests::styles_map_sgr_runs_to_char_cols` / `styles_survive_collapse_remap_and_trailing_clip` / `cursor_moves_dont_reset_style_and_join_runs` — span coords + palette mapping (basic/256)
  - `preview::tests::render_via_real_glow_strips_styles` — real glow: markers consumed, no ANSI, width respected; `render_reports_missing_binary` — injectable binary pins the error path
  - `preview_opens_tags_and_replaces_not_accumulates` (incl. styles captured), `preview_scratch_buffer_allowed`, `prompt_preview_command_routes`
  - `render::tests::preview_styles_paint_and_vanish_when_dirty` — glow colors land on cells; dirty preview drops stale spans
- Binary size: **9.3 MB** (gate < 50 MB ✅)
- Pty smoke (`/tmp/opencode/preview_smoke.py`, 40x100): ALL PASS ×7 checks — raw `**boldz**` painted, `:preview` shows `[preview]` + erased markers + rendered text + glow bg color (`48;5;`) in the frame, `:q` closes cleanly (no confirm), re-preview from source works, clean exit 0, no panic (note: ESC-then-colon must be sent as separate keystrokes — ESC+`:` decodes as Alt-`:`, a legacy-PTY fact worth knowing)

## Lazygit overlay (`C-g g` · `:git` · palette `lazygit`)

Issue #53. **Ctrl-G g** (third arm of the goto prefix: `d` def, `o` back,
`g` lazygit — action-mode `g g` too), **`:git`**, or palette **lazygit**
opens **lazygit** in a full-screen overlay over the whole editor area. The
tab row and status bar stay (the bar shows `lazygit: <dir>`).

- The overlay owns the **entire keyboard** — Esc, PgUp/PgDn, Alt-*, Ctrl-C
  all go to lazygit. The only pass-throughs: **Ctrl-\\** (hide; lazygit
  keeps running, `C-g g` brings it back — Ctrl-\ never *opens* it),
  Ctrl-Q (quit), Ctrl-O (suspend).
- **Quitting lazygit (q) auto-closes the overlay** — the pump notices the
  process exit every loop, drops the pane, and the editor repaints with
  `lazygit exited` in the status.
- One process: re-running the command while it runs just re-reveals a
  hidden overlay. Missing `lazygit` on PATH → status error, no pane.
- Files lazygit touched hit the existing external-change detection
  (reload / keep mine / cancel). Remappable/unbindable via `[keys]` name
  `lazygit`; the PTY resizes with the terminal keeping full-area geometry.

### Acceptance (lazygit overlay, issue #53)

- **621 unit + 14 flow + 25 lsp_flow** tests green (`cargo test`); zero warnings; clippy 95 (≤ 98 baseline).
- Behaviors:
  - `terminal.rs` generalized: `TerminalPane::spawn(rows, cols, cwd, program, args)` (`new` = the $SHELL wrapper); pane `label` field drives `status_line` (`lazygit: <cwd>`)
  - keymap: `(GotoPrefix, 'g') → OpenLazygit` (works from Ctrl-G *and* action `g` prefixes), palette entry `lazygit` (hint `C-g g`), bindable name `lazygit`; the handler consumes the prefix capture (GotoDef precedent) — bug found by pty smoke: a stale prefix swallowed the *next* key, so Ctrl-Q became a quiet cancel
  - spawn geometry = what layout draws: every view row (screen − status − echo − tab row); `apply_resize` keeps it exact
  - layout/draw/cursor/status: the visible overlay outranks the shell pane (which waits underneath); editor area goes to zero rows; clicks in the overlay area are no-ops via the existing terminal-region hit-test arm
  - key routing: `git_owns_key` (everything except Ctrl-Q/Ctrl-O) + `GitToggle` for both Ctrl-\ spellings (kitty `Char('\\')`, legacy 0x1c `Char('4')`) — with the overlay hidden, the legacy byte routes to the shell-pane toggle again
  - `git_pane_pump()` every loop iteration: EOF/try_wait → pane dropped + `lazygit exited` status
- Pinned behaviors (unit tests): `lazygit_tests::` spawn/cwd/height, re-reveal-not-respawn, `:git` routing, q-auto-close + status (real lazygit in a throwaway `git init` repo), full key-owner matrix, prefix-consumption regression; `keymap::goto_prefix_g_launches_lazygit` + palette/name tests; `render::git_overlay_layout_covers_whole_editor_area` + `apply_resize_resizes_git_overlay`; `terminal::spawn_missing_program_reports_spawn_error` + label status line
- Binary size: **9.5 MB** (gate < 50 MB ✅)
- Pty smoke (`/tmp/opencode/lazygit_smoke.py`, 30x110): **ALL PASS ×3** — `EDITOR-MARKER-42` in view, `C-g g` → lazygit's `Unstaged changes` panel + `Stage: <space>` keyhints painted, editor covered, `lazygit:` in status, `q` → overlay auto-closes (editor back, lazygit UI gone), `lazygit exited` in status, Ctrl-Q clean exit. **Negative control:** pre-fix (main) binary FAILs 6/9 checks — the overlay never appears for the same bytes.

## Status bar breadcrumb — center segment (`issue #56`)

The status bar is now **left = mode word (+sidebar tag) · center = breadcrumb ·
right = everything else** (buffers/branch/language/format/pane-status joined the
right-aligned block; `line:col 42% 2E 1W` stays pinned at the far right).

- Breadcrumb = **file path tail + enclosing symbol**, e.g.
  `src ▸ app.rs ▸ impl App ▸ fn handle_command` — computed from the
  tree-sitter tree already built for highlighting (deepest named ancestor walk,
  max 3 innermost symbols, kinds: fn/class/impl/struct/enum/trait). No LSP
  dependency; scratch buffers show path-only or nothing.
- **Narrow terminals clip the HEAD segments** — the deepest symbol always
  survives (orientation payoff); if nothing fits, the bar is exactly what it
  was before. Right-side core (pos/pct/diag) always wins; the right-side ext
  segments drop before it.
- New theme role `breadcrumbs` (built-in: status bar + DarkGray fg; override
  via `[theme.ui] breadcrumbs = {...}` or a theme file).
- Display-only: click-to-sibling dropdowns remain the header-row idea.

### Acceptance (status bar breadcrumb, issue #56)

- **631 unit + 14 flow + 25 lsp_flow** tests green; zero warnings; clippy 95 (≤98 baseline).
- Behaviors:
  - `syntax::SyntaxState::symbol_chain(pt)`: walk `descendant_for_point_range` up through ancestors, label structural kinds (function_item/class_definition/impl_item/… cross-grammar map, name-or-type field), cap 3 innermost, outer→inner; no tree → empty
  - `Document::breadcrumb()`: last two path segments (`file_name` + `parent().file_name()`) + symbol chain, " ▸ "-joined; caret ON a `fn` signature counts as inside it; blank lines between defs → path only; scratch → empty
  - `render::draw`: center placement centered in the gap between left end and right block start, min one space both sides; head segments clipped whole-segment until it fits; `breadcrumbs` style overlays `status_bar` (fg-only built-in so the bar background always survives); right side = ext+core when it fits, core-only when not
- Pinned behaviors (unit tests): `syntax::symbol_chain_*` (rust impl+method, top-level empty, python class+method, 3-cap innermost, no-tree); `document::breadcrumb_path_tail_and_symbol`; `render::breadcrumb_centered_between_mode_and_right_side` (incl. DarkGray paint), `breadcrumb_narrow_clips_head_keeps_deepest_symbol`, `scratch_buffer_has_no_breadcrumb`; `theme::breadcrumbs_role` (built-ins + merge-override + known-role note). Two pre-existing layout tests re-anchored for the new right-aligned segments (window widened; tab-row no-duplicate assert now targets the `[label]` form — the breadcrumb legitimately names the active file).
- Binary size: **9.5 MB** (gate < 50 MB ✅)
- Pty smoke (`/tmp/opencode/breadcrumb_smoke.py`, 30x110): **ALL PASS ×3** — `src ▸ crumb.rs ▸ fn alpha` painted at open, caret Down×4 onto `fn beta()` flips the segment to `crumb.rs ▸ fn beta` with the old symbol gone from the status row, right side still pinned, clean Ctrl-Q. **Negative control:** pre-fix main FAILs both breadcrumb paints (no `▸` anywhere).
