# plat — Phase 0 Design (MVP)

Date: 2026-11-01
Status: approved conversationally (sections 1–4), pending written-spec review
Parent doc: PLAN.md

## Purpose

`plat` (platypus) is a terminal text editor where **edit mode is home**:
typing always goes straight into the buffer. **Esc** enters a slim *action
mode* used to reduce keystrokes and finger stress — moving, selecting,
searching, acting on visible selections. Phase 0 is the hypothesis test:
live in this mode model as a daily driver for two weeks and decide whether
it feels faster than plain editing and not slower than Helix for everyday
moves.

Out of scope for Phase 0: LSP, tree-sitter, soft wrap, multi-cursor,
config-file keymaps, terminal overlay, sidebar, git gutters, Lua.

## Stack

- Rust, single crate.
- ratatui + crossterm (rendering/input), ropey (buffer), std only otherwise
  (file watching is mtime polling; TOML config arrives in Phase 1).

## 1. Mode model and keymap

Modes: `Edit` (home) · `Action` · `Prompt` (transient mini-modal for line
input: find, open, save-as, confirm). Esc from Prompt aborts back to the
previous mode; Enter commits. Status bar shows the current mode; cursor
shape indicates mode (bar = Edit, block = Action, via crossterm
`SetCursorStyle`).

The keymap is **data**: two binding tables plus globals mapping
`key → Command` enum. Tuning the keymap is a table edit (config-file
keymaps are Phase 1). `Ctrl-\` is reserved (future terminal overlay) and
must stay unbound.

### Global Ctrl keys (all modes)

| Key | Command |
|---|---|
| Ctrl-S | save |
| Ctrl-Z / Ctrl-Y | undo / redo |
| Ctrl-F | find (prompt) |
| Ctrl-P | open file (prompt) |
| Ctrl-W | close buffer (dirty → confirm) |
| Ctrl-Q, Ctrl-C | quit |
| Ctrl-O | **suspend to shell** (lazygit/difftool path) |

Edit mode additionally captures Ctrl-A / Ctrl-E for line start/end
(readline habit).

Ctrl-Z normally suspends a shell; plat captures it for undo — suspend
lives on Ctrl-O.

### Edit mode captures

Typing, Enter, Tab (2-space indent), Backspace, Delete, arrows, Home, End,
PgUp, PgDn, Ctrl-A, Ctrl-E, plus all global Ctrls. Everything else types
into the buffer.

### Action mode

Selection-first: act on ranges you can see. No hjkl, no operator-motions,
no counts.

| Keys | Action |
|---|---|
| Arrows, `w`/`b` | move; word jumps |
| `f`/`F` | jump to char on line (forward/backward) |
| `Space`, `Shift+arrows` | toggle char select / extend selection |
| `x` / `X` | select line / expand line |
| `Alt+-` then `"` `'` `(` `[` `{` | select inside quotes/brackets (textobjects) |
| `y` | yank selection |
| `p` / `P` | paste after / before |
| `d` | delete selection |
| `c` | change: delete selection, enter Edit mode |
| `/` | search prompt, live highlight; `n`/`N` cycle; selection becomes term |
| `u` / `U` | undo / redo |
| `:` | prompt: `w [path]`, `q`, `wq`, `e [path]` |

The core loop: Action → select → `c` → typing.

## 2. Architecture

Single crate, ~8 modules:

| Module | Job |
|---|---|
| `main.rs` | CLI (`plat [files...]`), terminal init/restore, panic hook that restores the terminal, event loop start |
| `app.rs` | State: `Vec<Document>`, active index, mode, viewport scroll, prompt state, status messages; owns the tick loop |
| `keymap.rs` | Edit/action/global binding tables: key → `Command` |
| `document.rs` | One file: `Rope`, dirty flag, path, cursor + `selections: Vec` (single selection anchor/head in Phase 0; Vec now so multi-cursor is a widening, not a rewrite), viewport, load/save/reload |
| `undo.rs` | Undo/redo as stacks of inverse rope edits (ropey edit events); history survives reload of same path |
| `prompt.rs` | Line-input mini-modal with history (up/down), returns commit/abort to app |
| `render.rs` | One immediate-mode `draw()`: viewport / status bar / prompt; renders only visible lines from the rope (this is where 100k+ files stay responsive) |
| `input.rs` | crossterm events → key decoding → keymap lookup; resize → viewport recompute |
| `watch.rs` | ~1s mtime poll (std, no notify crate); triggers reload/confirm flows |

Data flow: `event::poll(10 ms)` → decode → keymap → `Command` → mutate
Document/App → redraw. Rendering is immediate; ratatui's buffer diff
handles terminal updates. No async, no worker threads.

CLI: `plat` (empty buffer) or `plat file…` (each becomes a buffer, first
active). No stdin support in Phase 0.

## 3. Edge cases and error handling

- **Save failure**: status-bar error, buffer stays dirty; never lose buffer
  state on IO failure.
- **Dirty close/quit**: confirm prompt — save / discard / cancel.
- **External change** (lazygit round-trip): clean buffer → silent reload;
  dirty → confirm prompt (keep mine / reload / cancel). Undo history
  survives reload of the same path.
- **Encoding**: UTF-8 only; invalid UTF-8 refuses to open with a clear
  error (binary-file protection). Conversion is post-Phase-0.
- **Line endings**: detect on load, preserve on save.
- **Resize / tiny terminal**: recompute, clamp, never panic; minimal layout
  under ~20×10.
- **Suspend (Ctrl-O)**: `disable_raw_mode` + SIGTSTP; on resume
  re-initialize terminal and check all open file mtimes.
- **Panic hook**: restore terminal before printing panic.
- **Soft wrap**: out — no-wrap, horizontal scroll follows the cursor.

## 4. Testing and acceptance

- Unit tests:
  - document edit → undo → redo round-trips, including after reload
  - keymap dispatch: for each mode, key → expected `Command`
  - render slice math: offset/scroll on fake ropes — empty doc, cursor
    past EOL, one-column terminal
- One integration test: spawn `plat` against a temp file, simulate key
  events (type → Esc → action keys → save), assert file contents; catches
  terminal init/restore regressions.
- Acceptance = PLAN.md protocol: two weeks as sole daily driver with a
  punch-list file; week-one irritations become the Phase 0.1 keymap tuning
  pass. Success bar per PLAN.md: feels faster than plain editing, not
  slower than Helix for everyday moves; responsive on 100k+ line files.

## Phase 0 exit criteria

Open/save/type/Esc-action-mode/find/undo/redo/multi-buffer/suspend/reload —
working, ~3–5k lines, zero non-UTF-8/binary data loss, zero terminal
corruption on crash. Then two weeks of daily use before starting Phase 1.
