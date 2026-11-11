# plat — User Reference

The complete reference for **plat** (the platypus TUI editor): key bindings,
commands, and every configuration option. This file is the canonical user
documentation; it is written against the code (`src/keymap.rs`, `src/app.rs`,
`src/config.rs`, `src/theme.rs`) — if the README and this file disagree, this
file wins.

Table of contents: [Starting plat](#starting-plat) ·
[Modes](#modes) · [Keys](#keys) · [Commands](#commands) ·
[Remapping keys](#remapping-keys-keys) · [Configuration](#configuration) ·
[Themes](#themes) · [Feature notes](#feature-notes)

---

## Starting plat

```sh
plat [files...]          # open files (nonexistent paths open empty buffers)
plat <dir>               # open and browse a directory (tree sidebar at that root)
plat                     # no args: recent-files picker if history exists, else empty buffer
```

- **Bracketed paste** inserts text verbatim (newlines/tabs intact) as one undo step.
- Closing a dirty buffer (`:q`, `Ctrl-Q`, tab `×`) prompts: save / discard / cancel.
- External changes to open files are detected (mtime poll) and prompt:
  reload / keep mine / cancel.

### Environment variables

| Var | Effect |
|---|---|
| `PLAT_CONFIG=/path/config.toml` | Config file location override. If set and the file is missing, plat exits 1 with an error — never silently defaults. |
| `XDG_CONFIG_HOME` | Config dir base: `$XDG_CONFIG_HOME/platypus/config.toml` (default `~/.config/platypus/config.toml`). Theme files live in `themes/` next to it. |
| `XDG_DATA_HOME` | Recent-files store: `$XDG_DATA_HOME/platypus/recent.json` (default `~/.local/share/...`). |
| `COLORTERM=truecolor` | Read: 24-bit hex theme colors are used when present. |
| `PLAT_PANIC=1` | Dev switch: panic right after terminal init (verifies terminal restore). |
| `TERM` / kitty protocol | See [kitty-protocol notes](#kitty-protocol-vs-legacy-terminals). |

---

## Modes

- **Edit** (default): every printable key inserts text. The caret always
  types — mode can never trap your keystrokes.
- **Action**: navigation and editing verbs. `Esc` from Edit enters it;
  `Esc` returns to Edit (with a selection active, the first `Esc` only
  clears the selection). The status bar shows `EDIT` / `ACTION`.
- **Prompts**, **the sidebar**, **popups**, and **the terminal pane** each
  own the keyboard while visible; global Ctrl chords stay global
  (save/quit/undo/etc.) in every one of them.

---

## Keys

### Global Ctrl chords (all modes, including sidebar/prompts/terminal pane)

| Key | Action |
|---|---|
| `Ctrl-S` | save |
| `Ctrl-N` | new buffer (open-or-create; prompts for a name on save) |
| `Ctrl-Q` | quit (prompts per dirty buffer) |
| `Ctrl-Z` / `Ctrl-Y` | undo / redo |
| `Ctrl-C` / `Ctrl-X` / `Ctrl-V` | copy / cut / paste (CUA; `Ctrl-C` is deliberately **not** quit) |
| `Ctrl-W` / `Ctrl-Shift-W` | word forward / word back (`Ctrl-Shift-W` needs a kitty-protocol terminal) |
| `Ctrl-F` / `Ctrl-Shift-F` | find prompt / global workspace search (`Ctrl-Shift-F` needs kitty; legacy sends plain `Ctrl-F`) |
| `Ctrl-P` | fuzzy file picker (whole-tree lookup) |
| `Ctrl-B` | tree sidebar (toggle; browse lazily) |
| `Ctrl-T` | command palette |
| `Ctrl-O` | suspend plat to the shell (`fg` to return) |
| `Ctrl-G` then `d` / `o` | goto definition / jump back (two-step, works from any mode) |
| `Ctrl-Space` (or `Ctrl-@`) | LSP completion menu |
| `Ctrl-]` | LSP goto definition (kitty spelling; legacy terminals: `Ctrl-5` — both map from byte `0x1d`) |
| `Ctrl-\` | toggle embedded terminal pane |
| `Ctrl-<` / `Ctrl->` | buffer start / buffer end (kitty-protocol terminals; some terminals report the `Ctrl-4`/`Ctrl-6` spellings for the same chords) |
| `Ctrl-.` | LSP code actions (kitty-protocol terminals only; on legacy terminals the byte means buffer-end — use `:codeaction` or the palette) |
| `Ctrl-A` / `Ctrl-E` | line start / line end (**Edit mode** only, like readline) |

### Alt chords

| Key | Action |
|---|---|
| `Alt-H` / `Alt-L` | previous / next buffer (wraps; also with sidebar open) |
| `Alt-F` | global workspace search (`rg`) |
| `Alt-←` / `Alt-→` | shrink / grow sidebar (10–50 cols) — ignored when closed |
| `Alt-↑` / `Alt-↓` | grow / shrink terminal pane (when open) |
| `Alt-Enter` | **sidebar**: make the selected dir the new tree root |
| `Alt--` then a char | **action mode**: select inside an object (see [text objects](#two-step-verbs)) |

> `Alt-[` / `Alt-]` are deliberately never bindable: terminals send those as
> `ESC[` / `ESC]`, the CSI/OSC sequence prefixes — the keys never arrive.

### Edit mode

| Key | Action |
|---|---|
| any printable | insert (a typed char replaces an active selection) |
| `Enter` / `Tab` / `Backspace` / `Delete` | newline / indent / backspace / delete |
| `←↑↓→` | move; hold `Shift` to extend the selection |
| `Home` / `End` | line start / end (`Shift` extends) |
| `PageUp` / `PageDown` | page (Shift extends) |
| `Esc` | enter Action mode |

### Action mode

| Key | Action |
|---|---|
| `←↑↓→` / `Home` `End` / `PageUp` `PageDown` | move (`Shift` = extend selection) |
| `w` / `b` | word forward / back |
| `Space` | start char-wise selection |
| `x` / `X` | select line / expand selection by one line |
| `y` / `p` / `P` / `d` / `c` | yank / paste after / paste before / delete selection / change (delete + enter Edit) |
| `u` / `U` | undo / redo |
| `/` | find prompt · `n` / `N` next / previous match |
| `g` then `d` / `o` | goto definition / jump back |
| `f` / `F` then a char | jump to char right / left |
| `Alt--` then a char | select inside object: `(` `)` `[` `]` `{` `}` or any repeated delimiter (`"` `'` etc.) — the innermost pair around the caret, on the current line |
| `[` / `]` | previous / next LSP diagnostic |
| `:` | colon-command prompt |
| `Esc` | back to Edit mode |

`h/j/k/l` are intentionally **not** bound — arrows are the motion keys.

### Two-step verbs

`f`/`F` (jump char), `g` (goto), `Alt--` (object) wait for one more key; any
other key quietly cancels. `Ctrl-G` from any mode is the same goto prefix.

### While the sidebar is open (tree / picker / search)

| Key | Action |
|---|---|
| `↑` `↓` `PageUp` `PageDown` `Home` `End` | move the row cursor |
| any printable char | live filter (subsequence scoring) |
| `Backspace` | edit filter |
| `Enter` | open the selected file (and close); on a dir, toggle it; on the `..` row, re-root one level up |
| `Tab` | toggle the selected dir |
| `→` / `←` | expand / collapse-or-parent |
| `Alt-Enter` | re-root the tree at the selected dir (also moves the editor's cwd) |
| `Ctrl-R` | rescan the tree (tree keeps expansions + cursor) |
| `Esc` | first press clears the filter, second closes the sidebar |

Global Ctrl chords still work with the sidebar up: `Ctrl-B`/`Ctrl-P` switch
modes, `Ctrl-S` `Ctrl-N` `Ctrl-Z` `Ctrl-Y` `Ctrl-W` `Ctrl-Q` `Ctrl-T`
`Ctrl-O` `Ctrl-F`(`Ctrl-Shift-F`) as usual. Clicking the editor closes the
sidebar; the wheel scrolls the row cursor; drag the border to resize.

Re-opening the tree restores your last expansions and cursor (same root);
`:cd`/re-root starts a fresh tree. The tree filter matches only **loaded**
rows (open lists the root level; expanding lists that level) — whole-tree
lookup is the picker (`Ctrl-P`).

### Prompts (find/replace, save-as, `:` commands, confirms)

| Key | Action |
|---|---|
| printable / `Backspace` | edit |
| `Enter` | submit (or step) |
| `Esc` | cancel |
| `↑` / `↓` | search/command history |
| `Tab` | replace prompt: switch to the replacement field |
| `Ctrl-G` / `Ctrl-C` / `Ctrl-Q` | cancel (never quit mid-prompt) |

Find is a **regex** with live underline preview; a broken pattern keeps the
last good matches and shows the error. With a replacement set, plat walks
matches one at a time: `y` replace+next · `n` skip · `a` replace all (one
undo step) · `e` replace one and stop. `$1` /
`${name}` expand capture groups; `$$` is a literal `$`.

### Popup menus

- **Command palette** (`Ctrl-T`): type to filter (subsequence), `↑`/`↓`
  select, `Enter` run, `Esc` close. Hints reflect your `[keys]` remaps.
- **Completion** (`Ctrl-Space`): `Tab` down / `BackTab` up / `↑↓` /
  `Enter` accept / `Esc` close. The list is filtered client-side against
  the word before the caret; typing while open re-requests fresh items.
- **Code actions** (`Ctrl-.` / `:codeaction` / palette): same keyboard as
  completion.
- Recent-files and goto pickers reuse the palette widget (type to filter,
  Enter opens).

### Terminal pane (`Ctrl-\`)

All keys route to the shell except: `Ctrl-\` toggle, `Ctrl-Q` quit,
`Ctrl-O` suspend, `Alt-↑/↓` resize, `PageUp`/`PageDown` scroll the
2000-row scrollback. `:term` also toggles. The shell keeps its cwd,
history and scrollback across hiding.

### kitty-protocol vs legacy terminals

Shift-modified Ctrl chords (`Ctrl-Shift-W`, `Ctrl-Shift-F`) and `Ctrl-.`
(code actions) need a kitty-protocol terminal (kitty, foot, WezTerm,
Ghostty). Legacy terminals send the same byte as the unshifted chord, so
buffer-end there is `Ctrl-.`, and the byte for `Ctrl-,` is byte-identical
to `Ctrl-\` (0x1c) — on legacy terminals it is the terminal-pane toggle,
never buffer navigation. `Ctrl-]` arrives as `Ctrl-5` on legacy terminals.
`key_debug = true` echoes raw key events in the status bar to triage what
your terminal actually sends.

---

## Commands

### Colon commands (type `:` — echo line at the bottom)

| Command | Action |
|---|---|
| `:q` | close active buffer (prompts if dirty) |
| `:w` | save |
| `:wq` | save and close |
| `:w <path>` | save as (also `save_as` palette / prompt) |
| `:e <path>` | open a file by name (nonexistent paths open empty buffers) |
| `:cd` | print the current directory |
| `:cd <dir>` | change plat's cwd **and re-root the tree sidebar** there (`~` expands; LSP servers keep their old root until files reload) |
| `:term` | toggle the embedded terminal pane |
| `:preview` | markdown preview (needs `glow`) |
| `:format` | format the buffer via `[formatting]` or the LSP server |
| `:codeaction` | LSP code-action picker for the selection/caret |
| `:recent` | recent-files picker |

An unknown command prints `unknown command: <x>` in the echo line.

### Command palette (`Ctrl-T`) — every command, name and default key

| Name | Description | Default key |
|---|---|---|
| `save` | save current buffer | `C-s` |
| `save_as` | save with a new file name | — |
| `new_buffer` | add a new empty buffer | `C-n` |
| `close_buffer` | close active buffer | — |
| `next_buffer` | switch to next buffer | `A-l` |
| `prev_buffer` | switch to previous buffer | `A-h` |
| `undo` | undo last edit | `C-z` |
| `redo` | redo last undone edit | `C-y` |
| `find` | search (regex, live preview) | `C-f` |
| `open_file` | fuzzy file picker | `C-p` |
| `file_tree` | browse project files | `C-b` |
| `global_search` | search project (rg) | `A-f` |
| `markdown_preview` | preview markdown (glow) | — |
| `format_document` | format buffer via configured formatter or language server | — |
| `recent_files` | open a recently opened file | — |
| `code_action` | apply a code action from the language server | `C-.` |
| `toggle_terminal` | show/hide terminal pane | `C-\` |
| `goto_definition` | jump to definition (LSP) | `C-]` |
| `jump_back` | jump back in nav stack | — |
| `next_diagnostic` | next LSP diagnostic | `]` (action mode) |
| `prev_diagnostic` | previous LSP diagnostic | `[` (action mode) |
| `refresh_syntax` | reparse tree-sitter for the current buffer | — |
| `restart_lsp` | restart the LSP server for the current file's language | — |
| `yank` | copy selection (or line) | `C-c` |
| `paste` | paste at cursor | `C-v` |
| `command_mode` | colon commands (`:q` `:w` `:wq`) | `:` |
| `quit` | exit (prompts to save if dirty) | `C-q` |

`refresh_syntax` fixes stale highlighting after bulk edits; `restart_lsp`
is the fix when LSP looks wedged or after a `:cd`.

---

## Remapping keys (`[keys]`)

```toml
[keys]
"C-Space" = "select_line"   # overlay a binding
"C-x"     = "none"          # unbind (Nop — no fallthrough)
"x"       = "none"          # unbind an action-mode verb
```

- **Spec format:** `[C-][A-][S-]<key>` — `<key>` is a single character or a
  named token: `Space Enter Tab Esc Backspace Delete Up Down Left Right Home
  End PageUp PageDown` (case-insensitive token).
- Bindings apply over the built-in tables (including global Ctrl chords);
  palette hints follow your remaps.
- **Bindable names** (string → command, exactly as the palette names them
  plus these extras): `select_line expand_line begin_select_char jump_char
  jump_char_back object_prefix yank paste_after paste_before paste
  delete_selection change find find_next find_prev save undo redo
  refresh_syntax restart_lsp quit suspend command_palette save_as
  new_buffer close_buffer next_buffer prev_buffer open_sidebar_tree
  open_sidebar_picker global_search markdown_preview format_document
  recent_files code_action resize_sidebar.grow resize_sidebar.shrink
  toggle_terminal resize_terminal.grow resize_terminal.shrink indent
  insert_newline backspace delete enter_action_mode enter_edit_mode
  prev_diagnostic next_diagnostic completion goto_def jump_back`
  plus every motion as `move.<name>` and selection-extending as
  `move.<name>.extend`, where `<name>` is one of:
  `left right up down pageup pagedown word_next word_prev line_start
  line_end buffer_start buffer_end`.
  Example: `"C-Home" = "move.buffer_start"`.
- Runtime-internal actions (jump targets, object chars, sidebar row moves,
  prompt keys, `insert_char`, `nop`) are not bindable by design.
- Never bind `Alt-[` / `Alt-]` (arrive as escape-sequence prefixes, not keys).

---

## Configuration

File: `$XDG_CONFIG_HOME/platypus/config.toml` (default
`~/.config/platypus/config.toml`; `PLAT_CONFIG` overrides, and a missing
override file is an error). All sections and keys are optional — anything
absent keeps its default. A typo'd key is reported by name with a suggestion
and line number at startup; cosmetic numbers are clamped silently; a broken
theme never crashes the editor.

### `[editor]`

| Key | Type / default | Meaning |
|---|---|---|
| `line_numbers` | bool / `true` | gutter with line numbers (gutter width auto-scales with line count) |
| `indent_width` | int / `2` | Tab width (chars); clamped to ≤ 8 |
| `syntax` | bool / `true` | tree-sitter syntax highlighting |
| `theme` | string / `"dark"` | palette name: `"dark"`, `"light"`, or a `themes/<name>.toml` file |
| `key_debug` | bool / `false` | echo each raw key event into the status bar (binding triage) |
| `git_gutter` | bool / `true` | diff-marker column left of the gutter (see git notes) |
| `terminal_height` | int / `10` | terminal pane height in rows (clamped 3 … (rows−2)/2) |
| `format_on_save` | bool / `false` | run the formatter after every save (`:w`, `Ctrl-S`) |

### `[sidebar]`

| Key | Type / default | Meaning |
|---|---|---|
| `width` | int / `32` | initial sidebar width in columns (clamped 10–50) |
| `ignore` | list of gitignore-style patterns / `[]` | extra paths to skip during scans (matched root-relative; e.g. `"dist"`, `"*.min.js"`) |
| `icons` | bool / `true` | Nerd Font file-type icons per sidebar row (set `false` on terminals without a Nerd Font) |

Scans also honor the root's `.gitignore` and always skip the defaults:
`.git`, `node_modules`, `target`, `__pycache__`, `.egg-info`.

### `[ui]`

| Key | Type / default | Meaning |
|---|---|---|
| `mouse` | bool / `true` | capture clicks/wheel/drags. `false` leaves the mouse to the terminal (native drag-select still works) |

### `[lsp]` — language servers

```toml
[lsp]
rust = "rust-analyzer"            # string → whitespace-split argv
python = ["ruff", "server"]       # array → exact argv (args may contain spaces)
toml = false                      # disable this language's server entirely
```

`false`, `""`, and `[]` all mean disabled. A missing key attempts the
built-in default; a missing binary prints one `no server for <lang>` note
per session. `window/showMessage` text from servers surfaces as `lsp: …`.

Built-in defaults: rust `rust-analyzer` · c `clangd` · toml `taplo lsp stdio`
· markdown `marksman` · bash `bash-language-server start` · json
`vscode-json-language-server --stdio` · python `pylsp` · go `gopls` · lua
`lua-language-server`.

### `[formatting]` — external formatters

Same value semantics as `[lsp]`; **no built-in defaults** — a key's presence
overrides the LSP `textDocument/formatting` path: the whole buffer is piped
to the command's stdin and stdout replaces it as one undo step (UI never
blocks; a non-zero exit leaves the buffer untouched and reports stderr).

```toml
[lsp]        python = "pylsp"          # intelligence
[formatting] python = "ruff format -"  # formatting
             rust = false              # no formatting for rust at all
```

### `[theme.ui]` / `[theme.syntax]`

Live theme overrides (highest precedence, see [Themes](#themes)):

```toml
[theme.ui]
status_bar = { fg = "white", bg = "#1e1e2e" }
selection  = { bg = "dark-gray" }

[theme.syntax]
keyword = { fg = "#89b4fa", bold = true }
comment = "dark-gray"
```

---

## Themes

Two built-in palettes, `dark` (default) and `light`, plus per-user theme
files in the **`themes/`** directory next to the config
(`~/.config/platypus/themes/<name>.toml`), selected with
`[editor] theme = "<name>"`.

Precedence (later layers win): built-in dark base → theme file →
`[theme.ui]`/`[theme.syntax]` config overrides.

### Theme file format

```toml
[ui]
default         = { fg = "white", bg = "black" }  # base editor text fg/bg
line_no         = "#45475a"
current_line_no = { bold = true, underline = true }
status_mode     = { fg = "black", bg = "white" }  # the EDIT/ACTION word
status_bar      = { fg = "white", bg = "blue" }
tab_active      = { bold = true }
tab_inactive    = "dark-gray"
selection       = { bg = "#005f87" }
completion_sel  = { bg = "blue" }                 # menu highlight
menu_bg         = { bg = "dark-gray" }            # dropdown rectangle
sidebar_sel     = { bg = "blue" }
sidebar_dir     = { bold = true }                 # dir names + folder icons
find_match      = { underline = true }
diag_error      = "red"
diag_warn       = "yellow"
diag_info       = "dark-gray"
diag_hint       = "dark-gray"
palette_sel     = { bg = "blue" }

[syntax]
# scope names match tree-sitter node types; longest prefix wins
keyword  = "#ff0000"
function = { fg = "cyan", bold = true }
comment  = { fg = "dark-gray", italic = true }
```

Unknown keys are reported (`unknown theme ui key: …`), bad color values are
noted, a broken theme file falls back to the dark base — never a crash.

### Color grammar

| Form | Example | Meaning |
|---|---|---|
| Named | `"red"` | black white red green yellow blue magenta cyan |
| Bright | `"bright-red"` | bright variants; `light-*` is an alias for `bright-*` |
| Dark alias | `"dark-cyan"` / `"dark-yellow"` | xterm Cyan / Yellow (not bright) |
| Gray | `"gray"` / `"grey"` / `"dark-gray"` | gray tones |
| Reset | `"reset"` | terminal default |
| Hex | `"#005f87"` | 24-bit RGB (needs `COLORTERM=truecolor` for true emission) |
| Index | `240` | xterm 256-color index (integer form) |

A bare string/integer sets the foreground; use a table
`{ fg, bg, bold, italic, underline }` and omit what you don't want to
override — each key merges onto the base palette.

### Syntax scope names (built-in base)

`keyword` `function` `function.method` `method` `conditional` `repeat`
`type` `type.builtin` `string` `char` `number` `constant`
`boolean` `comment` `property` `field` `variable` `operator` `parameter`
`error` — plus any tree-sitter scope, addressable by longest-prefix match
(e.g. `function.method.call`).

File-type sidebar icon colors (the kind accents on tree rows) are built in,
not theme roles.

---

## Feature notes

### Buffers, tabs, status line

- Tab row appears only with **2+ buffers**: `[name+] ×` marks the active
  buffer (`+` = dirty, `×` click to close). `Alt-h`/`Alt-l` cycle with wrap.
- Status bar (second-to-last row): left = mode, sidebar tag, git branch
  `git(main)`, language, file format; right = `line:col` (or `N selected`),
  doc position `42%`/`Top`/`Bot`, diagnostic counts `2E 1W`.
- Echo line (last row): prompts, transient messages, the diagnostic message
  under the cursor.

### Search & replace

`/` find prompt (regex, live underlines), `n`/`N` cycle. `Tab` to the
replacement field → match-by-match walk (`y`/`n`/`a`/`e` above). `$1` /
`${name}` capture-group expansion in the replacement, `$$` literal `$`.

### Global search (`Alt-F`)

Runs `rg` (must be on PATH) in the sidebar root, streams hits live; type to
narrow (re-runs), Enter/click jumps to `file:line:col`. Smart-case, respects
gitignore semantics via rg itself.

### File sidebar

- **Tree (`Ctrl-B`)**: lazy per-directory scan — open lists the root level,
  first expand lists that level (cached; `Ctrl-R` rescans keeping
  expansions + cursor); re-rooting via `..` row (Enter / Alt+Enter on
  parents) or `Alt+Enter` on any dir (also moves cwd, like `:cd`).
  Connectors `├─ └─ │`, dirs end `/`, and every row carries a Nerd Font
  kind icon + per-kind color (`[sidebar] icons = false` to disable).
- **Picker (`Ctrl-P`)**: full-tree fuzzy scan (ranks every path), remembers
  its filter text across close/reopen.
- **Search (`Alt-F`)**: see global search.
- Filter: subsequence match, hit bytes underlined; tree filter matches
  loaded rows plus their ancestors.

### Git gutter (`[editor] git_gutter`)

Marker column left of the line-number gutter against `HEAD`: `+` added,
`~` changed, `-` deleted (badge column), with counts reflected in
diagnostics-free status. Branch name in the status bar (resolved per path,
cached; detached HEAD shows the short SHA).

### LSP

Diagnostics (gutter badges + counts + message under cursor, `[`/`]`
navigate), completion (`Ctrl-Space`, client-side filtering, live re-request
while typing), goto definition (`Ctrl-]`, `g d` / `Ctrl-G d`; jump back
`g o`), code actions (`Ctrl-.` kitty / `:codeaction` / palette; applies
`textDocument/codeAction` edits incl. multi-doc `documentChanges`), hover
and rename are on the roadmap. One server per (language, root), lazily
spawned, full-text sync.

### Markdown preview (`:preview`)

Renders the active `.md` buffer through **glow** (must be on PATH) into a
read-only preview buffer; re-previewing the same file replaces its preview
instead of stacking.

### Recently opened files

Every opened path is recorded (canonical, newest-first, cap 25) in
`$XDG_DATA_HOME/platypus/recent.json`; `:recent` / palette opens the picker;
launching plat with no arguments opens it when history exists.

### Mouse

Click places the caret; drag selects; Shift+click extends; Shift+drag does
the terminal's native selection. Tab click switches, `×` closes; sidebar row
click = Enter; border drag resizes the sidebar; wheel scrolls editor /
sidebar / palette. `[ui] mouse = false` hands the mouse back to the terminal.

### Suspend

`Ctrl-O` (or legacy action-mode `!`) stops plat, hands the real terminal to
your shell; `fg` resumes (re-enters alt-screen, repaints, re-announces the
kitty keyboard flags).

### Syntax highlighting

tree-sitter grammars out of the box: **Rust** `.rs` · **TOML** `.toml` ·
**Markdown** `.md` · **JSON** `.json` · **C** `.c` `.h` · **Bash** `.sh`
`.bash` · **Python** `.py` `.pyi` · **Go** `.go` · **Lua** `.lua`.
Unmapped scopes render default fg and are themeable by name.
