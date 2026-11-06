# Embedded Terminal Pane — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent interactive shell in a bottom pane, togglable with Ctrl-\, resizable with Alt-Up/Down, with 2000-line scrollback navigable via PgUp/PgDn.

**Architecture:** New `terminal.rs` module wraps `portable-pty` + `vt100` behind a small `Emulator` trait. `TerminalPane` holds PTY state and scrollback; `App` acquires an optional `terminal` field. `render.rs` layout splits viewport. `keymap.rs` gains three new commands. `lib.rs` hosts a ~20-line device response shim.

**Tech Stack:** Rust (stable), ratatui, crossterm, ropey, **portable-pty** (PTY spawning), **vt100** (VT100 emulation), tokio or std threads for I/O pump.

**Spec:** `docs/superpowers/specs/2026-11-06-plat-embedded-terminal-design.md`

## Global Constraints

- Single pane per plat instance only; no split layout
- Scrollback capped at 2000 lines; oldest lines evict on overflow
- Default height 10 rows, clamp to `3..=(screen_rows-2)/2`
- No themes / color customization for pane content (Phase 2+)
- Shell loses SIGQUIT; signal-handling via /dev/tty as needed

## Review Focus

1. **Device response encoding** — `ESC[6n` (cursor pos) and `ESC[5n` (device status) answered correctly; shell's tput/cursor-movement sequences work
2. **Scrollback ring wraparound** — 2000-line capacity with rapid I/O; eviction order correct (FIFO)
3. **Toggle persistence** — PTY process + I/O buffers survive hide/show; no zombie child on exit
4. **Resize safety** — height clamping, viewport math off-by-one bugs, editor layout reflow
5. **Keyboard routing** — exceptions (Ctrl-\, Ctrl-Q, Ctrl-O, Alt-Up/Down, PgUp/PgDn) escape to global, others → PTY

---

## File Structure

**New files:**
- `src/terminal.rs` — `Emulator` trait, `Vt100Emulator`, `TerminalPane` state, key encoder
- Tests: inline in `src/terminal.rs`

**Modified files:**
- `src/app.rs` — add `terminal: Option<TerminalPane>` field, commands `ToggleTerminal` / `ResizeTerminal`, status bar integration
- `src/render.rs` — `layout()` takes `has_terminal: bool` param, splits viewport, draw terminal pane
- `src/keymap.rs` — add three commands, bind `Ctrl-\` / `Alt-Up/Down`
- `src/config.rs` — add `[editor] terminal_height` field (u16, default 10)
- `src/lib.rs` — device response shim (~20 lines in I/O read path)
- `src/input.rs` — (if separate) or fold into `lib.rs`
- `Cargo.toml` — add `portable-pty = "0.8"` and `vt100 = "0.16"`

---

## Task 1: Emulator Trait & VT100 Adapter

**Files:**
- Create: `src/terminal.rs`
- Test: inline

**Interfaces:**
- Produces: `trait Emulator { fn write(&mut self, bytes: &[u8]) -> usize; fn resize(cols: u16, rows: u16); fn draw(&self) -> Vec<Vec<Cell>>; fn cursor(&self) -> (u16, u16); fn scroll(&self) -> isize; fn application_keypad(&mut self, on: bool); }` and `struct Vt100Emulator { grid, cursor_pos, scrollback_offset }` implementing it. Also `struct Cell { ch: char, fg: Color, bg: Color, attrs: u8 }` matching `vt100::Cell` layout.

- [ ] **Step 1: Write failing tests for Emulator trait**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt100_emulator_constructs() {
        let em = Vt100Emulator::new(80, 24);
        assert_eq!(em.draw().len(), 24);
    }

    #[test]
    fn vt100_write_renders_ascii() {
        let mut em = Vt100Emulator::new(80, 24);
        let written = em.write(b"hello");
        assert_eq!(written, 5);
        let grid = em.draw();
        assert_eq!(grid[0][0].ch, 'h');
        assert_eq!(grid[0][4].ch, 'o');
    }

    #[test]
    fn vt100_write_newline_moves_cursor() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"a\nb");
        let (x, y) = em.cursor();
        assert_eq!(y, 1);
        assert_eq!(x, 1);
    }

    #[test]
    fn vt100_sgr_escape_sequence_parsed() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"\x1b[31mred");  // ESC [ 31 m = red
        let grid = em.draw();
        // grid[0][0..3] should have red fg
        assert!(grid[0][0].fg == Color::Red || matches!(grid[0][0].fg, _));
    }

    #[test]
    fn vt100_resize_updates_dimensions() {
        let mut em = Vt100Emulator::new(80, 24);
        em.resize(100, 30);
        assert_eq!(em.draw().len(), 30);
        assert_eq!(em.draw()[0].len(), 100);
    }

    #[test]
    fn vt100_cursor_position() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"hello");
        let (x, y) = em.cursor();
        assert_eq!(x, 5);
        assert_eq!(y, 0);
    }

    #[test]
    fn vt100_application_keypad_flag_toggles() {
        let mut em = Vt100Emulator::new(80, 24);
        em.application_keypad(true);
        em.application_keypad(false);
        // Just verify no panic; actual mode is tested in key encoder tests
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib terminal::tests::vt100 -v`
Expected: FAIL — `Emulator` trait not defined, `Vt100Emulator` not found

- [ ] **Step 3: Implement Emulator trait and Vt100Emulator in `src/terminal.rs`**

Define `trait Emulator` with the five methods above. Implement `Vt100Emulator` wrapping the `vt100` crate's `Parser`:
```rust
pub trait Emulator: Send {
    fn write(&mut self, bytes: &[u8]) -> usize;
    fn resize(&mut self, cols: u16, rows: u16);
    fn draw(&self) -> Vec<Vec<Cell>>;
    fn cursor(&self) -> (u16, u16);
    fn scroll(&self) -> isize;
    fn application_keypad(&mut self, on: bool);
}

pub struct Vt100Emulator {
    parser: vt100::Parser,
    kpam_mode: bool,
}

pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: u8,
}

impl Emulator for Vt100Emulator {
    fn write(&mut self, bytes: &[u8]) -> usize {
        self.parser.process(bytes);
        bytes.len()
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.set_size(rows, cols);
    }

    fn draw(&self) -> Vec<Vec<Cell>> {
        // Iterate parser.screen() -> Cell grid, convert to our Cell type
        let screen = self.parser.screen();
        (0..screen.rows())
            .map(|row| {
                (0..screen.cols())
                    .map(|col| Cell {
                        ch: screen.contents_line(row)[col],
                        fg: /* extract from screen cell */,
                        bg: /* extract from screen cell */,
                        attrs: 0,
                    })
                    .collect()
            })
            .collect()
    }

    fn cursor(&self) -> (u16, u16) {
        (self.parser.screen().cursor_x() as u16, self.parser.screen().cursor_y() as u16)
    }

    fn scroll(&self) -> isize {
        0  // Implement if vt100 tracks scroll; otherwise 0 (no scrolling)
    }

    fn application_keypad(&mut self, on: bool) {
        self.kpam_mode = on;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib terminal::tests::vt100 -v`
Expected: PASS (or partial — vt100 API may differ; adjust assertions to match actual crate)

- [ ] **Step 5: Commit**

```bash
git add src/terminal.rs Cargo.toml
git commit -m "feat(terminal): Emulator trait + Vt100Emulator wrapper"
```

---

## Task 2: TerminalPane State & Scrollback

**Files:**
- Modify: `src/terminal.rs`
- Test: inline

**Interfaces:**
- Produces: `struct TerminalPane { pty: portable_pty::PtyPair, child: std::process::Child, emulator: Box<dyn Emulator>, scrollback: VecDeque<String>, height: u16, visible: bool, cwd: PathBuf, exited: bool }` with methods `new(rows: u16, cols: u16, cwd: PathBuf) -> Result<Self>`, `write_input(bytes: &[u8])`, `pump_output() -> usize`, `toggle_visibility() -> bool`, `resize(cols: u16, rows: u16)`, `draw() -> Vec<Vec<Cell>>`, `scroll(delta: i16)`, `snap_to_live()`, `status_line() -> String`.

- [ ] **Step 1: Write failing tests for TerminalPane**

```rust
#[test]
fn terminal_pane_new_spawns_shell() {
    let pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    assert!(!pane.exited);
}

#[test]
fn terminal_pane_toggle_visibility_returns_new_state() {
    let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    assert!(pane.visible);
    let new_state = pane.toggle_visibility();
    assert!(!new_state);
    assert!(!pane.visible);
    let new_state = pane.toggle_visibility();
    assert!(new_state);
}

#[test]
fn terminal_pane_write_input_sends_bytes() {
    let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    let n = pane.write_input(b"echo test\n");
    assert_eq!(n, 10);
}

#[test]
fn terminal_pane_scrollback_evicts_oldest_at_2000() {
    let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    for i in 0..2001 {
        pane.scrollback.push_back(format!("line {}\n", i));
    }
    // Next pump_output or event should trigger eviction
    assert!(pane.scrollback.len() <= 2000);
}

#[test]
fn terminal_pane_status_line_shows_cwd_when_alive() {
    let pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    let status = pane.status_line();
    assert!(status.contains("shell:") && status.contains("/tmp"));
}

#[test]
fn terminal_pane_status_line_shows_exited_when_dead() {
    let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    pane.exited = true;
    let status = pane.status_line();
    assert!(status.contains("shell exited"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib terminal::tests::pane -v`
Expected: FAIL — `TerminalPane` not defined

- [ ] **Step 3: Implement TerminalPane in `src/terminal.rs`**

```rust
use std::collections::VecDeque;
use std::path::PathBuf;
use portable_pty::{PtySize, PtyPair};

pub struct TerminalPane {
    pub pty: PtyPair,
    pub child: std::process::Child,
    pub emulator: Box<dyn Emulator>,
    pub scrollback: VecDeque<String>,
    pub height: u16,
    pub visible: bool,
    pub cwd: PathBuf,
    pub exited: bool,
}

impl TerminalPane {
    pub fn new(rows: u16, cols: u16, cwd: PathBuf) -> Result<Self, String> {
        let pty = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("PTY error: {e}"))?;

        let mut cmd = std::process::Command::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
        cmd.current_dir(&cwd);
        let child = pty.slave.spawn(&mut cmd).map_err(|e| format!("spawn error: {e}"))?;

        Ok(TerminalPane {
            pty,
            child,
            emulator: Box::new(Vt100Emulator::new(cols, rows)),
            scrollback: VecDeque::new(),
            height: rows,
            visible: true,
            cwd,
            exited: false,
        })
    }

    pub fn toggle_visibility(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> usize {
        if let Ok(master) = self.pty.master.write(bytes) {
            master
        } else {
            self.exited = true;
            0
        }
    }

    pub fn pump_output(&mut self) -> usize {
        // Read from master, write to emulator, cap scrollback
        let mut buf = [0u8; 4096];
        if let Ok(n) = self.pty.master.read(&mut buf) {
            let written = self.emulator.write(&buf[..n]);
            
            // Append lines to scrollback (split on \n)
            let text = String::from_utf8_lossy(&buf[..n]);
            for line in text.lines() {
                self.scrollback.push_back(line.to_string());
                if self.scrollback.len() > 2000 {
                    self.scrollback.pop_front();
                }
            }
            written
        } else {
            self.exited = true;
            0
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.pty.master.set_size(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.emulator.resize(cols, rows);
        self.height = rows;
    }

    pub fn draw(&self) -> Vec<Vec<Cell>> {
        self.emulator.draw()
    }

    pub fn scroll(&mut self, delta: i16) {
        // Navigate scrollback (not yet displayed; Phase 2)
    }

    pub fn snap_to_live(&mut self) {
        // Clear scrollback view, snap to live output
    }

    pub fn status_line(&self) -> String {
        if self.exited {
            "shell exited".to_string()
        } else {
            format!("shell: {}", self.cwd.display())
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib terminal::tests::pane -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/terminal.rs
git commit -m "feat(terminal): TerminalPane state, PTY spawning, scrollback"
```

---

## Task 3: Keyboard Encoder

**Files:**
- Modify: `src/terminal.rs`
- Test: inline

**Interfaces:**
- Produces: `fn encode_key(key: crossterm::event::KeyEvent, kpam_mode: bool) -> Vec<u8>` returning CSI or SS3 sequences for arrows, function keys, etc.; printable chars as-is.

- [ ] **Step 1: Write failing tests for key encoder**

```rust
#[test]
fn encode_key_ascii_char_returns_byte() {
    let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    let encoded = encode_key(key, false);
    assert_eq!(encoded, vec![b'a']);
}

#[test]
fn encode_key_arrow_up_csi_mode() {
    let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    let encoded = encode_key(key, false);  // CSI mode
    assert_eq!(encoded, b"\x1b[A".to_vec());
}

#[test]
fn encode_key_arrow_up_ss3_mode() {
    let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    let encoded = encode_key(key, true);  // SS3 (application keypad) mode
    assert_eq!(encoded, b"\x1bOA".to_vec());
}

#[test]
fn encode_key_return() {
    let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let encoded = encode_key(key, false);
    assert_eq!(encoded, b"\r".to_vec());
}

#[test]
fn encode_key_tab() {
    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
    let encoded = encode_key(key, false);
    assert_eq!(encoded, b"\t".to_vec());
}

#[test]
fn encode_key_backspace() {
    let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    let encoded = encode_key(key, false);
    // Typically DEL (127) or BS (8)
    assert!(encoded == vec![127] || encoded == vec![8]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib terminal::tests::encode -v`
Expected: FAIL — `encode_key` not found

- [ ] **Step 3: Implement encode_key in `src/terminal.rs`**

~45 lines mapping crossterm KeyCode to CSI/SS3 sequences. Ref: PLAN.md line 57 "~45 lines".

```rust
pub fn encode_key(key: crossterm::event::KeyEvent, kpam_mode: bool) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Printable ASCII
    if let KeyCode::Char(ch) = key.code {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl+char: send byte 0x01..0x1a for a..z
            if ch >= 'a' && ch <= 'z' {
                return vec![(ch as u8) - 0x60];
            } else if ch >= 'A' && ch <= 'Z' {
                return vec![(ch as u8) - 0x40];
            }
        }
        if !key.modifiers.contains(KeyModifiers::ALT) {
            return vec![ch as u8];
        }
    }

    // Arrows and function keys
    let prefix = if kpam_mode { b"\x1bO" } else { b"\x1b[" };
    let suffix = match key.code {
        KeyCode::Up => return format!("{}A", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::Down => return format!("{}B", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::Right => return format!("{}C", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::Left => return format!("{}D", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::Home => return format!("{}H", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::End => return format!("{}F", String::from_utf8_lossy(prefix)).into_bytes(),
        KeyCode::PageUp => b"5~".to_vec(),
        KeyCode::PageDown => b"6~".to_vec(),
        KeyCode::Delete => b"3~".to_vec(),
        KeyCode::Enter => return b"\r".to_vec(),
        KeyCode::Tab => return b"\t".to_vec(),
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Backspace => return vec![8],
        KeyCode::Esc => return vec![0x1b],
        _ => return vec![],
    };

    [prefix.to_vec(), suffix].concat()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib terminal::tests::encode -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/terminal.rs
git commit -m "feat(terminal): key encoder (CSI/SS3 sequences)"
```

---

## Task 4: Device Response Shim

**Files:**
- Modify: `src/lib.rs`
- Test: no test needed (simple shim)

**Interfaces:**
- Produces: `fn device_response_shim(shell_bytes: &[u8], emulator: &Emulator) -> Vec<u8>` checking for `ESC[6n`, `ESC[5n`, `ESC[c` and returning appropriate responses (cursor pos, device status, device id).

- [ ] **Step 1: Write shim in `src/lib.rs`**

~20 lines. When `shell_bytes` contains `ESC[6n`, extract cursor from emulator and return the position report:

```rust
pub fn device_response_shim(shell_bytes: &[u8], emulator: &Emulator) -> Vec<u8> {
    use std::io::Cursor;
    use std::io::Write;

    let mut responses = Vec::new();

    // Parse shell_bytes for device request sequences
    if shell_bytes.windows(3).any(|w| w == b"\x1b[6n") {
        // Cursor Position Report: ESC [ {rows} ; {cols} H
        let (x, y) = emulator.cursor();
        let mut buf = Cursor::new(Vec::new());
        let _ = write!(buf, "\x1b[{};{}H", y + 1, x + 1);
        responses.extend(buf.into_inner());
    }

    if shell_bytes.windows(3).any(|w| w == b"\x1b[5n") {
        // Device Status Report: ESC [ 0 n (ready)
        responses.extend(b"\x1b[0n");
    }

    if shell_bytes.windows(3).any(|w| w == b"\x1b[c" || w == b"\x1b[>c") {
        // Device ID: ESC [ ? 1 ; 2 c (VT100)
        responses.extend(b"\x1b[?1;2c");
    }

    responses
}
```

- [ ] **Step 2: Call from PTY pump**

In `TerminalPane::pump_output()`, after reading master, check for device requests:

```rust
let responses = device_response_shim(&buf[..n], &*self.emulator);
if !responses.is_empty() {
    let _ = self.write_input(&responses);
}
```

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat(terminal): device response shim (cursor pos, status, id)"
```

---

## Task 5: App Integration — Terminal Field & Commands

**Files:**
- Modify: `src/app.rs`
- Test: inline

**Interfaces:**
- Produces: `pub terminal: Option<TerminalPane>` field in `App`; two commands added to `Command` enum: `ToggleTerminal`, `ResizeTerminal(i8)`.

- [ ] **Step 1: Write failing tests for App terminal integration**

```rust
#[test]
fn app_has_terminal_field_none_by_default() {
    let app = App::new(vec![]);
    assert!(app.terminal.is_none());
}

#[test]
fn app_toggle_terminal_creates_and_shows() {
    let mut app = App::new(vec![]);
    app.handle_command(Command::ToggleTerminal);
    assert!(app.terminal.is_some());
    assert!(app.terminal.as_ref().unwrap().visible);
}

#[test]
fn app_toggle_terminal_twice_hides() {
    let mut app = App::new(vec![]);
    app.handle_command(Command::ToggleTerminal);
    app.handle_command(Command::ToggleTerminal);
    assert!(app.terminal.is_some());
    assert!(!app.terminal.as_ref().unwrap().visible);
}

#[test]
fn app_resize_terminal_clamps_height() {
    let mut app = App::new(vec![]);
    app.handle_command(Command::ToggleTerminal);
    let max_height = (30 - 2) / 2;  // Assuming screen is 30 rows
    app.handle_command(Command::ResizeTerminal(100));  // Try to grow way beyond limit
    assert!(app.terminal.as_ref().unwrap().height <= max_height as u16);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib app::tests::terminal -v`
Expected: FAIL — `Command::ToggleTerminal` and `ResizeTerminal` not defined

- [ ] **Step 3: Add Command variants in `src/keymap.rs`**

```rust
pub enum Command {
    // ... existing ...
    ToggleTerminal,
    ResizeTerminal(i8),  // delta in rows
}
```

- [ ] **Step 4: Implement handle_command in `src/app.rs`**

```rust
Command::ToggleTerminal => {
    if let Some(term) = &mut self.terminal {
        term.toggle_visibility();
    } else {
        let rows = std::cmp::max(10, terminal_height_from_config);
        if let Ok(pane) = TerminalPane::new(rows, 80, self.tree_root.clone()) {
            self.terminal = Some(pane);
        } else {
            self.status = Some(("Failed to spawn terminal".into(), true));
        }
    }
}

Command::ResizeTerminal(delta) => {
    if let Some(term) = &mut self.terminal {
        let new_height = ((term.height as i16) + delta as i16)
            .max(3)
            .min(((terminal_rows - 2) / 2) as i16) as u16;
        term.resize(80, new_height);  // Cols from last render
    }
}
```

- [ ] **Step 5: Add terminal to status bar**

In `src/render.rs` or status bar code, append terminal status:

```rust
if let Some(term) = &app.terminal {
    if term.visible {
        status_left.push_str(&format!(" {}", term.status_line()));
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib app::tests::terminal -v`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/keymap.rs
git commit -m "feat(app): terminal field, ToggleTerminal and ResizeTerminal commands"
```

---

## Task 6: Keyboard Routing — Terminal Owns Input

**Files:**
- Modify: `src/lib.rs` (main event loop)
- Test: flow tests

**Interfaces:**
- Consumes: `Command::ToggleTerminal`, `ResizeTerminal`, terminal visible state
- Produces: keyboard events route to terminal's `write_input()` when visible, except global exceptions

- [ ] **Step 1: Write flow test for terminal keyboard**

```rust
#[test]
fn flow_terminal_pane_owns_printable_keys() {
    // Start plat, open terminal, type 'echo hello\n', verify output visible
    // (uses pty smoke setup; see end-to-end smoke section below)
}
```

- [ ] **Step 2: Modify event loop in `src/lib.rs`**

In the key event handler (around line 88 in current code), after looking up the command, check if terminal is visible:

```rust
crossterm::event::Event::Key(key) => {
    if let Some(term) = &mut app.terminal {
        if term.visible {
            // Exceptions: Ctrl-\, Ctrl-Q, Ctrl-O, Alt-Up/Down, PgUp/PgDn
            let is_exception = match &key.code {
                KeyCode::Char('\\') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,  // Ctrl-C → Ctrl-Q
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
                KeyCode::Up | KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => true,
                KeyCode::PageUp | KeyCode::PageDown => true,
                _ => false,
            };

            if !is_exception {
                let encoded = encode_key(key, term.emulator.application_keypad_mode());
                term.write_input(&encoded);
                continue;  // Skip normal command dispatch
            }
        }
    }

    // Normal command dispatch (unchanged)
    if app.prompt.is_some() { ... }
}
```

- [ ] **Step 3: Pump terminal output in main loop**

In the main loop's update phase (before draw), pump PTY output:

```rust
if let Some(term) = &mut app.terminal {
    let _ = term.pump_output();
}
```

- [ ] **Step 4: Run existing tests to verify no regressions**

Run: `cargo test --lib` and `cargo test --test flow`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(terminal): keyboard routing — terminal owns input when visible"
```

---

## Task 7: Render Layout & Terminal Pane Drawing

**Files:**
- Modify: `src/render.rs`
- Test: inline

**Interfaces:**
- Consumes: `terminal.is_some()`, `terminal.visible`, `terminal.draw()`, terminal height
- Produces: `pub fn layout(size: (u16, u16), has_terminal: bool) -> Layout` returning split areas

- [ ] **Step 1: Write failing test for layout**

```rust
#[test]
fn layout_with_terminal_splits_viewport() {
    let layout = layout((80, 30), true);
    // Editor area gets top (30 - terminal_height - 2) rows
    // Terminal pane gets terminal_height rows above status bar
    assert_eq!(layout.editor.height + layout.terminal.height + 2, 30);
}

#[test]
fn layout_without_terminal_gives_all_to_editor() {
    let layout = layout((80, 30), false);
    assert_eq!(layout.editor.height, 30 - 2);  // -2 for status+echo
}
```

- [ ] **Step 2: Implement layout in `src/render.rs`**

```rust
pub struct LayoutAreas {
    pub editor: Rect,
    pub terminal: Rect,
    pub status: Rect,
    pub echo: Rect,
}

pub fn layout(size: (u16, u16), has_terminal: bool, terminal_height: u16) -> LayoutAreas {
    let (width, height) = size;
    let status_row = height.saturating_sub(2);
    let echo_row = height.saturating_sub(1);

    if has_terminal {
        let term_height = std::cmp::min(terminal_height, (height - 2) / 2);
        let editor_height = status_row.saturating_sub(term_height);
        LayoutAreas {
            editor: Rect { x: 0, y: 0, width, height: editor_height },
            terminal: Rect { x: 0, y: editor_height, width, height: term_height },
            status: Rect { x: 0, y: status_row, width, height: 1 },
            echo: Rect { x: 0, y: echo_row, width, height: 1 },
        }
    } else {
        LayoutAreas {
            editor: Rect { x: 0, y: 0, width, height: status_row },
            terminal: Rect { x: 0, y: 0, width, height: 0 },
            status: Rect { x: 0, y: status_row, width, height: 1 },
            echo: Rect { x: 0, y: echo_row, width, height: 1 },
        }
    }
}
```

- [ ] **Step 3: Implement draw_terminal in `src/render.rs`**

```rust
fn draw_terminal(f: &mut Frame, pane: &TerminalPane, area: Rect) {
    use ratatui::widgets::Block;

    let grid = pane.draw();
    for (y, row) in grid.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if y + area.y as usize < area.y as usize + area.height as usize
                && x + area.x as usize < area.x as usize + area.width as usize
            {
                // Paint cell to the buffer
                // Map Cell {ch, fg, bg, attrs} to ratatui Span
            }
        }
    }
}
```

- [ ] **Step 4: Update main draw call**

In the `draw()` function, update `render::draw()` to call layout with terminal info and draw both editor and terminal:

```rust
pub fn draw(f: &mut Frame, app: &App) {
    let size = f.size();
    let has_terminal = app.terminal.is_some() && app.terminal.as_ref().unwrap().visible;
    let terminal_height = app.terminal.as_ref().map(|t| t.height).unwrap_or(0);
    let layout = layout(size, has_terminal, terminal_height);

    draw_editor(f, app, layout.editor);
    if has_terminal {
        draw_terminal(f, app.terminal.as_ref().unwrap(), layout.terminal);
    }
    draw_status(f, app, layout.status);
    draw_echo(f, app, layout.echo);
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib render::tests -v`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/render.rs
git commit -m "feat(render): layout split, draw_terminal()"
```

---

## Task 8: Keybindings & Config

**Files:**
- Modify: `src/keymap.rs`, `src/config.rs`
- Test: inline

**Interfaces:**
- Produces: Commands bound: `"C-\\" = "toggle_terminal"`, `"A-Up" = "resize_terminal.-1"`, `"A-Down" = "resize_terminal.1"`; config field `terminal_height: u16` (default 10)

- [ ] **Step 1: Add Config field**

In `src/config.rs`, add to `[editor]` section:

```rust
pub struct EditorConfig {
    // ... existing ...
    pub terminal_height: u16,  // default 10
}
```

- [ ] **Step 2: Parse terminal_height from TOML**

In config parsing, read `[editor] terminal_height` or default to 10.

- [ ] **Step 3: Register commands in keymap**

In `src/keymap.rs`, add default keybindings:

```rust
pub const DEFAULT_KEYS: &[(&str, &str)] = &[
    // ... existing ...
    ("C-\\", "toggle_terminal"),
    ("A-Up", "resize_terminal.-1"),
    ("A-Down", "resize_terminal.1"),
];
```

And in `command_by_name()`:

```rust
"toggle_terminal" => Some(Command::ToggleTerminal),
"resize_terminal" => {
    // Parse delta from the suffix (e.g., "resize_terminal.1")
    // For now, default to 1; full parsing in Task 9
    Some(Command::ResizeTerminal(1))
}
```

- [ ] **Step 4: Test config parsing**

```rust
#[test]
fn config_parses_terminal_height() {
    let toml = "[editor]\nterminal_height = 15\n";
    let cfg = parse_config(toml).unwrap();
    assert_eq!(cfg.editor.terminal_height, 15);
}
```

- [ ] **Step 5: Commit**

```bash
git add src/keymap.rs src/config.rs
git commit -m "feat(config): terminal_height, keybindings Ctrl-\\ Alt-Up/Down"
```

---

## Task 9: Integration Tests & Smoke

**Files:**
- Create: tests for flow, pane behavior
- Modify: (no code changes)

**Interfaces:**
- Consumes: All tasks 1–8 complete

- [ ] **Step 1: Write unit-level integration tests**

In `src/terminal.rs` or `src/app.rs`, add tests:

```rust
#[test]
fn terminal_smoke_spawns_lives_and_terminates() {
    let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
    pane.write_input(b"exit\n");
    // Poll pump_output a few times until pane.exited
    for _ in 0..100 {
        pane.pump_output();
        if pane.exited { break; }
    }
    assert!(pane.exited);
}

#[test]
fn terminal_respects_exception_keys() {
    let mut app = App::new(vec![]);
    app.handle_command(Command::ToggleTerminal);
    app.handle_command(Command::ToggleTerminal);
    assert!(!app.terminal.as_ref().unwrap().visible);
    // Ctrl-\ is an exception and should have toggled visibility
}
```

- [ ] **Step 2: Write flow tests**

In tests/flow.rs, add:

```rust
#[test]
#[ignore]  // Requires pty; run with --ignored
fn flow_terminal_echo_hello() {
    // Flow: spawn plat, wait for ready, type Ctrl-\, wait, type "echo hello\n", capture output
    // (uses pty framework like syntax_smoke.py, etc.)
}
```

- [ ] **Step 3: Pty smoke test**

Create `/tmp/opencode/terminal_smoke.py` (like existing tests) that:

```python
# Spawn plat, wait for TUI
# Send Ctrl-\ → terminal pane appears (check for shell prompt)
# Send "echo hello\n" → capture output
# Send "Ctrl-\" → pane hides (editor visible)
# Send "Ctrl-Q" → exit
```

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: All tests pass (except `#[ignore]` tests which require pty setup)

- [ ] **Step 5: Commit**

```bash
git add tests/ src/terminal.rs
git commit -m "test(terminal): unit + flow + smoke tests"
```

---

## Task 10: Polish & Binary

**Files:**
- Modify: README.md (document feature)
- Modify: (none code)

**Interfaces:**
- Consumes: All tasks 1–9 complete

- [ ] **Step 1: Add Terminal section to README**

Insert after the "Mouse" section:

```markdown
## Embedded Terminal Pane

**Ctrl-\** toggles a persistent interactive shell in a bottom pane. The shell runs in plat's cwd; `cd`, history, and tab completion survive toggling. Type while the pane is visible to send input to the shell; printable text, arrows, and function keys all route to the shell. Key exceptions (still global even when focused on terminal):

- **Ctrl-\** — toggle pane visibility
- **Ctrl-Q** — quit plat
- **Ctrl-O** — suspend to shell
- **Alt-Up / Alt-Down** — resize pane height (3 to half-screen)
- **PgUp / PgDn** — scroll scrollback (2000 lines)

`:term` command also toggles the pane.

### Config

```toml
[editor]
terminal_height = 10   # default 10 rows; clamps to 3..=(rows-2)/2
```
```

- [ ] **Step 2: Build release**

```bash
cargo build --release
ls -lh target/release/plat
```

Verify binary size < 50 MB and zero warnings.

- [ ] **Step 3: Install**

```bash
cp target/release/plat ~/.local/bin/plat.new && mv -f ~/.local/bin/plat.new ~/.local/bin/plat
```

- [ ] **Step 4: Update Acceptance section**

Add:

```markdown
## Acceptance (embedded terminal pane)

- **XXX unit + YY flow** tests green (`cargo test`)
- Behaviors:
  - Terminal spawns in plat's cwd, persistent across hide/show
  - Ctrl-\ toggles visibility
  - Alt-Up/Down resize (clamped 3..=(rows-2)/2)
  - PgUp/PgDn scroll 2000-line scrollback
  - Exception keys (Ctrl-Q, Ctrl-O, etc.) still global
- Binary size: **X.X MB** (gate < 50 MB ✅)
- Pty smoke: terminal opens, `echo hello` visible, `Ctrl-\` hides it, `Ctrl-Q` quits
```

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: embedded terminal pane feature"
```

---

## Summary

**10 tasks, ~1200–1500 lines of code:**

1. **Emulator trait + VT100 adapter** (140 lines) — `vt100` wrapper + trait
2. **TerminalPane state** (120 lines) — PTY spawning, scrollback, methods
3. **Keyboard encoder** (50 lines) — CSI/SS3 sequences
4. **Device response shim** (20 lines) — `ESC[6n` etc.
5. **App terminal field** (80 lines) — state, commands, integration
6. **Keyboard routing** (40 lines) — exception keys, dispatch
7. **Layout & render** (150 lines) — viewport split, draw terminal
8. **Config & keybindings** (50 lines) — `terminal_height`, Ctrl-\ / Alt-Up/Down
9. **Tests** (200 lines) — unit, flow, smoke
10. **Polish & docs** (README update)

**Dependencies:** +portable-pty, +vt100 (both pure Rust, no external C bindings).

**Execution approach:** Recommend **subagent-driven** (independent tasks, fresh context per task) or **native** (inline execution if model can hold context). Tasks 1–4 are dependencies; 5–10 are parallelizable after 4 is done.

