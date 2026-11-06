# Embedded Terminal Pane — Design

**Work order #12.** A **persistent interactive shell** in a **bottom pane**, resizable, togglable with **Ctrl-\**.
Resolves the "Built-in Terminal" contradiction (PLAN.md line 40): terminal stays open and retains session
(cd, history, completion, scrollback) when toggled hidden.

## Goals

1. **Persistent shell session** — spawn `/bin/sh` (or `$SHELL`), cwd = plat's cwd, keep it running across toggle-hide
2. **Resizable pane** — bottom, above status bar; default 10 rows, clamp to `3..=(rows-2)/2`
3. **Scrollback** — 2000-line capacity, PgUp/PgDn navigate; printable input snaps to live/bottom
4. **VT100 rendering** — output streams through `vt100` parser; small `Emulator` trait allows future libghostty-vt swap
5. **Keyboard ownership** — pane owns most keys while visible; exceptions: `Ctrl-\` (toggle), `Ctrl-Q` (quit), `Ctrl-O` (suspend), `Alt-Up/Down` (resize), `PgUp/PgDn` (scroll)
6. **Device responses** — shim handles `ESC[6n` (cursor pos), `ESC[5n` (device status), `ESC[c` / `ESC[>c` (id)

## Scope

### In Scope

- **Core:** PTY spawning, shell I/O, vt100 parse, rendering
- **UX:** Ctrl-\ toggle, Alt-Up/Down resize, PgUp/PgDn scrollback
- **Keybinding:** most keystrokes route to shell while visible; escape-able via `:q` or Ctrl-\
- **Config:** default height in `[editor] terminal_height = 10`
- **Status:** pane gains shell working directory and exit status ("shell: ~/work" or "shell exited")

### Out of Scope — Phase 2+

- **Pipe-through:** selection → stdin → replace stdout (spec line 66, deferred)
- **Scroll resize sync:** smooth resize when scrollback shrinks
- **Multiple shells:** one pane per plat instance only
- **Themes:** pane uses terminal's native colors only (no SGR override)
- **Copy/paste menu:** integration with plat's clipboard

## Architecture

### New module: `src/terminal.rs`

- **`struct Emulator`** — trait: `write(&mut self, bytes) -> usize`, `resize(cols, rows)`, `draw() -> Grid<Cell>`, `cursor() -> (x, y)`, `scroll() -> isize`, `application_keypad(bool)`
  - Concrete: `Vt100Emulator` wrapping the `vt100` crate's parser; cell model matches Vt100 Cell (fg/bg/attrs)
- **`struct TerminalPane`** — state, buffer, scrollback ring
  - `PTY: portable_pty::PtyPair`; `shell: Child` (spawned process)
  - `emulator: Box<dyn Emulator>` (trait object for vt100 ⇄ future swaps)
  - `scrollback: VecDeque<String>` (2000 lines, stored as String for simplicity)
  - `height: u16`, `visible: bool`, `cwd: PathBuf`, `exited: bool`
  - Methods: `new(rows, cols)`, `toggle_visibility()`, `resize(cols, rows)`, `write_input(bytes)`, `pump_output()`, `draw()`, `scroll(delta)`, `snap_to_live()`
- **Keyboard encoder** — `encode_key(key: Key) -> Vec<u8>` (~45 lines) mapping crossterm Key to CSI/SS3 sequences; respects `application_keypad()` mode

### Modified: `src/app.rs`

- Add: `pub terminal: Option<TerminalPane>` field (opened lazily on first `:term` / `Ctrl-\`)
- `handle_command(Command::ToggleTerminal)` — create if missing, toggle visible
- `handle_command(Command::ResizeTerminal(delta))` — call `terminal.resize()`
- `handle_key_in_terminal(key)` — encode key, send to PTY
- Status bar additions: when terminal exists, append `"shell: {cwd}"` or `"shell exited"` on the left

### Modified: `src/render.rs`

- `pub fn layout(size: (u16, u16), has_terminal: bool) -> Layout` — split viewport:
  - Editor area gets top `height - terminal_height - 2` rows (status bar + echo line stay at bottom)
  - Terminal pane gets `terminal_height` rows just above status bar (if visible)
- Render call: `draw_terminal(pane, area)` paints the grid

### Modified: `src/keymap.rs`

- Add: `Command::ToggleTerminal`, `Command::ResizeTerminal(i8)` — delta in rows
- Keybindings:
  - `"C-\\" = "toggle_terminal"`
  - `"A-Up" = "resize_terminal.-1"`
  - `"A-Down" = "resize_terminal.1"`
- When terminal is visible and focused: all keys except `Ctrl-\`, `Ctrl-Q`, `Ctrl-O`, `Alt-Up/Down`, `PgUp/PgDn` route to `handle_key_in_terminal()`

### Modified: `src/input.rs` or `src/lib.rs`

- **Device response shim** (~20 lines in the read path): when shell sends `ESC[6n`, reply with `CSI{cursor_y};{cursor_x}H`; etc.

### Modified: `Cargo.toml`

- Add: `portable-pty = "0.8"`, `vt100 = "0.16"`

### Modified: `src/config.rs`

- Add: `[editor] terminal_height = 10` (u16, default 10)

## Keymaps & Behavior

### Global (outside terminal pane)

- `Ctrl-\` — create terminal if missing, toggle `visible` state
- `:term` — same as `Ctrl-\`

### In Terminal Pane (while visible)

- **All printable chars, arrows, etc.** → encode and send to PTY
- **Exceptions (still global):**
  - `Ctrl-\` — toggle pane visibility
  - `Ctrl-Q` — quit plat (even in terminal)
  - `Ctrl-O` — suspend plat (even in terminal)
  - `Alt-Up` / `Alt-Down` — resize pane (not sent to shell)
  - `PgUp` / `PgDn` — scroll scrollback (not sent to shell)
- **Paste** — if terminal is open and visible, paste → PTY; otherwise → active buffer

### Scrollback Navigation

- `PgUp` — scroll up 10 lines in scrollback ring
- `PgDn` — snap to live (if scrolled), or no-op if already at bottom
- Printable input (typing) → snap to live first, clear scrollback viewport, send char to shell

## Acceptance Criteria

1. ✅ Shell spawns in plat's cwd; `pwd` in terminal shows plat's cwd
2. ✅ `Ctrl-\` toggles pane visibility; process stays alive
3. ✅ `Alt-Up` / `Alt-Down` resize pane; status bar relayout happens; clamped to `3..=(rows-2)/2`
4. ✅ Scrollback captures 2000 lines; `PgUp/PgDn` navigate; typing snaps to live
5. ✅ Device responses: `ESC[6n` → cursor position; shell works correctly
6. ✅ Status bar shows `shell: ~/work` (cwd) when visible; `shell exited` if child died
7. ✅ All Phase 1 tests still pass; new terminal tests cover spawning, toggle, resize, I/O
8. ✅ Keybinding exceptions work: `Ctrl-Q` quits even when focused on terminal; `PgUp` does not go to shell
9. ✅ Pty smoke: terminal opens, `echo hello` prints visible text, `Ctrl-\` hides it, `Ctrl-\` shows it, `Ctrl-Q` quits

## Review Focus

1. **Device response parsing** — shell's `tput cols` / `tput lines` or cursor-save-restore sequences must work correctly
2. **Scrollback ring wraparound** — 2000 lines with rapid I/O; old lines evict correctly
3. **Toggle hide/show** — PTY process state (I/O buffers, running shell) survives hide; no zombie child
4. **Resize clamp bounds** — minimum 3 rows, maximum `(screen_rows - 2) / 2`; off-by-one in math
5. **Keyboard encoding** — Arrow keys, function keys in both SS3 and CSI modes; modifiers (Shift, Ctrl, Alt) handled

## Testing Strategy

### Unit tests (src/terminal.rs, inline)

- `test_vt100_emulator_constructs` — instantiation
- `test_emulator_write_renders_text` — plain text + newlines → grid
- `test_emulator_sgr_colors` — escape sequences parsed
- `test_emulator_cursor_position` — tracks x, y
- `test_scrollback_ring_evicts_oldest` — 2001st line removes 1st
- `test_key_encoder_ascii` — 'a' → `[97]`
- `test_key_encoder_arrow_csi` — arrow keys in CSI mode
- `test_key_encoder_arrow_ss3` — arrow keys in SS3 mode
- `test_device_response_cursor_position` — shell `ESC[6n` → reply

### Flow tests (tests/flow.rs)

- `flow_terminal_pane_toggles_visible` — `Ctrl-\` twice, assert visible state
- `flow_terminal_spawns_in_plat_cwd` — open terminal, `pwd` via PTY, verify
- `flow_terminal_resize_clamps` — Alt-Up many times, verify height stays in range

### Pty smoke (once shipped)

- `flow_terminal_pane_echoes_user_input` — type `echo hello`, see output
- Verify shell exits cleanly when plat quits

## Deferred

- **Phase 2:** Pipe-through (selection → `|` command → replace with output)
- **Phase 2:** Themes (color override via `[theme.terminal]`)
- **Phase 2:** Multiple panes / split layout
- **Phase 3:** Lua hooks in PTY output (like neovim's TermOpen)
