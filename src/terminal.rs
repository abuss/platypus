use portable_pty::{CommandBuilder, PtyPair, PtySize};
use ratatui::style::Color;
use std::io::{Read, Write};
use std::path::PathBuf;

/// Color-able cell in the terminal grid
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: u8,
}

/// Terminal emulator abstraction trait
pub trait Emulator: Send {
    /// Process bytes of VT100 escape sequences and content.
    /// Returns the number of bytes written.
    fn write(&mut self, bytes: &[u8]) -> usize;

    /// Resize the terminal to the given dimensions (cols x rows).
    fn resize(&mut self, cols: u16, rows: u16);

    /// Draw the current screen state as a 2D grid of cells.
    fn draw(&self) -> Vec<Vec<Cell>>;

    /// Get the current cursor position (x, y).
    fn cursor(&self) -> (u16, u16);

    /// Rows of vt100-native scrollback to show above the live screen
    /// (0 = live). Clamped to the available ring by the emulator.
    fn set_scrollback(&mut self, rows: usize);

    /// Rows currently scrolled back (0 = live).
    fn scrollback(&self) -> usize;

    /// Get the scroll offset (0 if no scrollback).
    fn scroll(&self) -> isize;

    /// Set application keypad mode on/off.
    fn application_keypad(&mut self, on: bool);

    /// Current keypad/cursor application mode (DECCKM).
    fn keypad_mode(&self) -> bool;
}

/// Scrollback capacity: oldest lines evict past this.
pub const SCROLLBACK_LIMIT: usize = 2000;

/// Terminal pane with PTY and scrollback.
///
/// The child is spawned into a PTY running $SHELL (else /bin/sh) with
/// `cwd` as its working directory. The master reader is polled
/// non-blocking: `pump_output` never waits for output.
pub struct TerminalPane {
    pub pty: PtyPair,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub emulator: Box<dyn Emulator>,
    pub height: u16,
    pub visible: bool,
    pub cwd: PathBuf,
    pub exited: bool,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
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

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(&cwd);
        let child = pty
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn error: {e}"))?;

        // portable-pty's reader is a raw fd with blocking semantics; on Linux a
        // read() on a quiet shell would block the UI thread. Set O_NONBLOCK on
        // the master before cloning reader/writer fds (flag is shared by dups).
        Self::set_reader_nonblocking(&*pty.master)?;

        let reader = pty
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader error: {e}"))?;
        let writer = pty
            .master
            .take_writer()
            .map_err(|e| format!("writer error: {e}"))?;

        Ok(TerminalPane {
            pty,
            child,
            emulator: Box::new(Vt100Emulator::new(cols, rows)),
            height: rows,
            visible: true,
            cwd,
            exited: false,
            reader,
            writer,
        })
    }

    pub fn toggle_visibility(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> usize {
        match self.writer.write(bytes).and_then(|_| self.writer.flush()) {
            Ok(()) => bytes.len(),
            Err(_) => {
                self.exited = true;
                0
            }
        }
    }

    /// Read any available PTY output without blocking. Returns bytes consumed.
    /// Marks the pane `exited` when the shell is gone (EOF or child reaped).
    pub fn pump_output(&mut self) -> usize {
        // Check if the child process has already exited.
        if !self.exited {
            if let Ok(Some(_)) = self.child.try_wait() {
                self.exited = true;
            }
        }
        let mut buf = [0u8; 4096];
        match self.reader.read(&mut buf) {
            Ok(0) => {
                self.exited = true;
                0
            }
            Ok(n) => {
                self.emulator.write(&buf[..n]);
                
                // Scan for device queries and generate replies after emulator has processed.
                // ponytail: if a query and text both arrive in one buffer, cursor reports
                // the post-text position. Real shells wait for replies before sending more,
                // so this is rare; per-query slicing (parse, reply, feed, parse next, ...)
                // upgrades Phase-1 to handle text-after-query correctly.
                let responses = device_response_shim(&buf[..n], &*self.emulator);
                if !responses.is_empty() {
                    let _ = self.write_input(&responses);
                }
                n
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => 0,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => 0,
            Err(_) => {
                // Reader died (shell closed the pty) or read failed: treat as exit.
                self.exited = true;
                0
            }
        }
    }

    /// Set O_NONBLOCK on the reader fd so pump_output can return WouldBlock
    /// instead of blocking. The reader from try_clone_reader is a duplicate
    /// of the master fd, so we set the flag on the master's raw fd directly;
    /// O_NONBLOCK is a file-status flag shared by all dup'd fds.
    fn set_reader_nonblocking(master: &dyn portable_pty::MasterPty) -> Result<(), String> {
        let fd = master
            .as_raw_fd()
            .ok_or_else(|| "PTY master has no raw fd".to_string())?;
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(format!("fcntl F_GETFL: {}", std::io::Error::last_os_error()));
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(format!("fcntl F_SETFL: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.pty.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.emulator.resize(cols, rows);
        self.height = rows;
    }

    /// The pane's visible grid. When scrolled back, vt100's own scrollback
    /// ring IS the view: `set_scrollback` rewires what `screen.cell()` (and
    /// therefore this draw) returns — cell-accurate, colors included.
    pub fn draw(&self) -> Vec<Vec<Cell>> {
        self.emulator.draw()
    }

    /// Rows currently scrolled back from the live view (0 = live).
    pub fn view_scroll(&self) -> usize {
        self.emulator.scrollback()
    }

    pub fn scroll(&mut self, delta: i16) {
        // Negative delta = scroll back into history (PgUp), positive = toward
        // live. vt100 clamps against the real ring size internally.
        let cur = self.emulator.scrollback() as isize;
        self.emulator.set_scrollback((cur - delta as isize).max(0) as usize);
    }

    pub fn snap_to_live(&mut self) {
        self.emulator.set_scrollback(0);
    }

    /// Current application cursor/keypad mode (DECCKM), for encode_key.
    pub fn keypad_mode(&self) -> bool {
        self.emulator.keypad_mode()
    }

    pub fn status_line(&self) -> String {
        let base = if self.exited {
            "shell exited".to_string()
        } else {
            format!("shell: {}", self.cwd.display())
        };
        if self.view_scroll() > 0 {
            format!("{base}  (scrollback +{})", self.view_scroll())
        } else {
            base
        }
    }
}

/// VT100 terminal emulator using the vt100 crate
pub struct Vt100Emulator {
    parser: vt100::Parser,
    kpam_mode: bool,
}

impl Vt100Emulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        // Third arg = vt100's NATIVE scrollback ring: cell-accurate rows with
        // attrs/colors as they scroll off the top. 0 disables it — that was
        // the old hand-rolled-line-ring workaround.
        let parser = vt100::Parser::new(rows, cols, SCROLLBACK_LIMIT);
        Self {
            parser,
            kpam_mode: false,
        }
    }
}

/// Convert vt100 Color to ratatui Color
fn convert_color(vt_color: vt100::Color) -> Color {
    match vt_color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => {
            // Map common color indices
            match idx {
                0 => Color::Black,
                1 => Color::Red,
                2 => Color::Green,
                3 => Color::Yellow,
                4 => Color::Blue,
                5 => Color::Magenta,
                6 => Color::Cyan,
                7 => Color::White,
                8 => Color::DarkGray,
                9 => Color::LightRed,
                10 => Color::LightGreen,
                11 => Color::LightYellow,
                12 => Color::LightBlue,
                13 => Color::LightMagenta,
                14 => Color::LightCyan,
                15 => Color::White,
                _ => Color::Indexed(idx),
            }
        }
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
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
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        
        (0..rows)
            .map(|row| {
                (0..cols)
                    .map(|col| {
                        let cell_opt = screen.cell(row, col);
                        match cell_opt {
                            Some(vt_cell) => {
                                let contents = vt_cell.contents();
                                let ch = contents.chars().next().unwrap_or(' ');
                                Cell {
                                    ch,
                                    fg: convert_color(vt_cell.fgcolor()),
                                    bg: convert_color(vt_cell.bgcolor()),
                                    attrs: 0,
                                }
                            }
                            None => Cell {
                                ch: ' ',
                                fg: Color::Reset,
                                bg: Color::Reset,
                                attrs: 0,
                            },
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn cursor(&self) -> (u16, u16) {
        let (row, col) = self.parser.screen().cursor_position();
        (col, row)
    }

    fn set_scrollback(&mut self, rows: usize) {
        self.parser.set_scrollback(rows);
    }

    fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    fn scroll(&self) -> isize {
        0
    }

    fn application_keypad(&mut self, on: bool) {
        // ponytail: kpam_mode field kept for interface compatibility; actual mode
        // is tracked by vt100::Parser internally via DECCKM — keypad_mode() is
        // the source of truth.
        self.kpam_mode = on;
    }

    fn keypad_mode(&self) -> bool {
        self.parser.screen().application_cursor()
    }
}

/// Device response shim: intercepts VT100 device queries and generates replies.
/// Scans raw bytes for ESC[6n (cursor position), ESC[5n (device status),
/// ESC[c and ESC[>c (device id) and returns appropriate responses.
/// 
/// NOTE: Query sequences arriving split across pump boundaries are not detected
/// (e.g., ESC in one read, [6n in the next). Rare in practice since queries are
/// typically atomic, but upgrade to stateful detection if this becomes frequent.
fn device_response_shim(shell_bytes: &[u8], emulator: &dyn Emulator) -> Vec<u8> {
    let mut responses = Vec::new();

    // ESC[6n: Cursor Position Report → ESC[{row};{col}R (1-based)
    if shell_bytes.windows(4).any(|w| w == b"\x1b[6n") {
        let (x, y) = emulator.cursor();
        use std::io::Write;
        let mut buf = Vec::new();
        let _ = write!(buf, "\x1b[{};{}R", y + 1, x + 1);
        responses.extend(buf);
    }

    // ESC[5n: Device Status Report → ESC[0n (ready)
    if shell_bytes.windows(4).any(|w| w == b"\x1b[5n") {
        responses.extend(b"\x1b[0n");
    }

    // ESC[c: Device ID → ESC[?1;2c (VT100 with AVO)
    if shell_bytes.windows(3).any(|w| w == b"\x1b[c") {
        responses.extend(b"\x1b[?1;2c");
    }
    
    // ESC[>c: Secondary DA → ESC[?1;2c (VT100 with AVO)
    if shell_bytes.windows(4).any(|w| w == b"\x1b[>c") {
        responses.extend(b"\x1b[?1;2c");
    }

    responses
}

/// Encode a crossterm KeyEvent to terminal input bytes (CSI/SS3 sequences).
pub fn encode_key(key: crossterm::event::KeyEvent, kpam_mode: bool) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Printable ASCII: Char code without modifiers or with Ctrl
    if let KeyCode::Char(ch) = key.code {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl+char: send byte 0x01..0x1a for a..z or A..Z
            if ch >= 'a' && ch <= 'z' {
                return vec![(ch as u8) - 0x60];
            } else if ch >= 'A' && ch <= 'Z' {
                return vec![(ch as u8) - 0x40];
            }
        }
        // Encode char as UTF-8 (handles non-ASCII: é, CJK, etc.)
        let mut buf = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buf).as_bytes().to_vec();
        // Without Alt: send UTF-8 bytes as-is
        if !key.modifiers.contains(KeyModifiers::ALT) {
            return bytes;
        }
        // With Alt: ESC-prefix the UTF-8 bytes (Alt+x = ESC + UTF-8)
        if key.modifiers.contains(KeyModifiers::ALT) {
            let mut result = vec![0x1b];
            result.extend(bytes);
            return result;
        }
    }

    // Special keys: arrows, function keys, etc.
    match key.code {
        // Alt+Enter → ESC then CR (shell sees Alt-Enter; encode_key's normal
        // Char path doesn't cover Enter, so handle explicitly here)
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => vec![0x1b, b'\r'],
        // Arrows in SS3 mode (application keypad)
        KeyCode::Up if kpam_mode => vec![0x1b, b'O', b'A'],
        KeyCode::Down if kpam_mode => vec![0x1b, b'O', b'B'],
        KeyCode::Right if kpam_mode => vec![0x1b, b'O', b'C'],
        KeyCode::Left if kpam_mode => vec![0x1b, b'O', b'D'],
        // Arrows in CSI mode (cursor keys)
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        // Other CSI sequences
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Backspace => vec![127],
        KeyCode::Esc => vec![0x1b],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_scrollback_scrolls_cells_view() {
        // vt100's OWN scrollback: feed the emulator directly (no shell), 6 lines
        // into a 4-row screen → 2 lines held in the native ring. (No trailing
        // \r\n: a final LF at the bottom row would scroll one extra line.)
        let mut pane = TerminalPane::new(4, 20, PathBuf::from("/tmp")).unwrap();
        pane.emulator.write(b"l1\r\nl2\r\nl3\r\nl4\r\nl5\r\nl6");
        pane.snap_to_live();
        let row0: String = pane.draw()[0].iter().map(|c| c.ch).collect();
        assert!(row0.starts_with("l3"), "live top row after 2 lines scrolled off: {row0:?}");
        pane.scroll(-2); // like PgUp: 2 rows back
        let top: String = pane.draw()[0].iter().map(|c| c.ch).collect();
        assert!(top.starts_with("l1"), "scrolled view shows oldest line at top: {top:?}");
        pane.snap_to_live();
        let row0: String = pane.draw()[0].iter().map(|c| c.ch).collect();
        assert!(row0.starts_with("l3"), "snap_to_live restores the live view: {row0:?}");
    }

    #[test]
    fn native_scrollback_preserves_colors() {
        // The whole point of the native ring: scrolled-off cells keep their SGR.
        let mut pane = TerminalPane::new(4, 20, PathBuf::from("/tmp")).unwrap();
        pane.emulator.write(b"\x1b[31mRED\x1b[0m\r\nl2\r\nl3\r\nl4\r\nl5");
        pane.snap_to_live();
        pane.scroll(-1);
        let grid = pane.draw();
        let red = grid.iter().flatten().find(|c| c.ch == 'R');
        assert_eq!(red.map(|c| c.fg), Some(Color::Red),
            "scrolled-off cell must keep its red fg");
    }

    #[test]
    fn pane_view_and_status_report_native_offset() {
        let mut pane = TerminalPane::new(4, 20, PathBuf::from("/tmp")).unwrap();
        pane.emulator.write(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n");
        pane.snap_to_live();
        pane.scroll(-2);
        assert_eq!(pane.view_scroll(), 2, "view_scroll mirrors the vt100 offset");
        assert!(pane.status_line().contains("scrollback +2"), "{}", pane.status_line());
        pane.scroll(-1000); // vt100 clamps to available history; must stay positive
        assert!(pane.view_scroll() > 0);
        pane.snap_to_live();
        assert_eq!(pane.view_scroll(), 0);
    }

    #[test]
    fn vt100_emulator_constructs() {        let em = Vt100Emulator::new(80, 24);
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
        assert_eq!(x, 2);
    }

    #[test]
    fn vt100_sgr_escape_sequence_parsed() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"\x1b[31mred"); // ESC [ 31 m = red (ANSI color index 1 → ratatui Color::Red)
        let grid = em.draw();
        // grid[0][0..3] should have red fg: vt100 maps ANSI 31 → idx 1 → convert_color maps idx 1 → Color::Red
        assert_eq!(grid[0][0].fg, Color::Red);
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

    // Device response shim tests
    #[test]
    fn device_response_shim_6n_returns_cursor_report() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"abc");
        let responses = device_response_shim(b"\x1b[6n", &em);
        // After "abc", cursor is at (3, 0). Report is 1-based: ESC[1;4R
        assert_eq!(responses, b"\x1b[1;4R".to_vec());
    }

    #[test]
    fn device_response_shim_6n_text_then_query_same_buffer() {
        // CRITICAL: production case where text + query arrive in one pump buffer.
        // Emulator must be fed BEFORE scanning for queries, so cursor reflects post-text state.
        let mut em = Vt100Emulator::new(80, 24);
        let buf = b"hello\x1b[6n";
        em.write(buf);
        let responses = device_response_shim(buf, &em);
        // After "hello" (5 chars), cursor is at (5, 0). Report is 1-based: ESC[1;6R
        assert_eq!(responses, b"\x1b[1;6R".to_vec());
    }

    #[test]
    fn device_response_shim_6n_after_newline() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"a\nbc");
        let responses = device_response_shim(b"\x1b[6n", &em);
        // After "a\nbc", cursor on row 1 (2nd row, 1-based), col 2 (3rd char, 1-based)
        // Verify format: starts with ESC[, ends with R, contains semicolon
        assert_eq!(responses[0], 27); // ESC
        assert_eq!(responses[1], 91); // [
        assert_eq!(responses[responses.len() - 1], b'R');
        // Verify the full response parses as ESC[N;NR
        let s = String::from_utf8_lossy(&responses);
        assert!(s.starts_with("\x1b[") && s.ends_with("R"), "malformed cursor report: {}", s);
        assert!(s.contains(";"), "cursor report missing semicolon: {}", s);
    }

    #[test]
    fn device_response_shim_5n_returns_status() {
        let em = Vt100Emulator::new(80, 24);
        let responses = device_response_shim(b"\x1b[5n", &em);
        assert_eq!(responses, b"\x1b[0n".to_vec());
    }

    #[test]
    fn device_response_shim_c_returns_device_id() {
        let em = Vt100Emulator::new(80, 24);
        let responses = device_response_shim(b"\x1b[c", &em);
        assert_eq!(responses, b"\x1b[?1;2c".to_vec());
    }

    #[test]
    fn device_response_shim_greater_c_returns_device_id() {
        let em = Vt100Emulator::new(80, 24);
        let responses = device_response_shim(b"\x1b[>c", &em);
        assert_eq!(responses, b"\x1b[?1;2c".to_vec());
    }

    #[test]
    fn device_response_shim_no_query_returns_empty() {
        let em = Vt100Emulator::new(80, 24);
        let responses = device_response_shim(b"hello world", &em);
        assert!(responses.is_empty());
    }

    #[test]
    fn device_response_shim_multiple_queries() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"hi");
        let responses = device_response_shim(b"\x1b[6n\x1b[5n", &em);
        // After "hi" (2 chars), cursor at (2, 0). 1-based: row 1, col 3.
        // Response: ESC[1;3R (cursor) + ESC[0n (status)
        assert_eq!(responses, b"\x1b[1;3R\x1b[0n".to_vec());
    }

    #[test]
    fn vt100_application_keypad_flag_toggles() {
        let mut em = Vt100Emulator::new(80, 24);
        em.application_keypad(true);
        em.application_keypad(false);
        // Just verify no panic; actual mode is tested in key encoder tests
    }

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
    fn terminal_pane_pump_output_feeds_emulator_without_blocking() {
        let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
        pane.write_input(b"echo hi\n");
        // Bound the wait: pump until the echo lands or 2s elapse, never hang.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            pane.pump_output();
            let row0: String = pane.draw()[0].iter().map(|c| c.ch).collect();
            if row0.contains("hi") || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let row0: String = pane.draw()[0].iter().map(|c| c.ch).collect();
        assert!(row0.contains("hi"), "shell output not rendered: {row0:?}");

        // Now the shell is quiet: pump_output must return promptly (nonblocking
        // reader) and must NOT mistake WouldBlock for the shell exiting. Any
        // still-buffered echo/prompt is drained first; the timed check covers
        // only the final, quiet-shell pump. If a read() ever blocked, these
        // asserts blow past their ceilings instantly.
        let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if pane.pump_output() == 0 {
                break;
            }
            assert!(std::time::Instant::now() < drain_deadline, "pump_output blocked/hung draining");
        }
        assert!(!pane.exited, "pump must not mark a live shell exited");
        let start = std::time::Instant::now();
        let n = pane.pump_output();
        let elapsed = start.elapsed();
        assert_eq!(n, 0, "quiet shell must yield no further output");
        assert!(elapsed < std::time::Duration::from_millis(100), "quiet pump slow/blocked: {elapsed:?}");
        assert!(!pane.exited, "WouldBlock must not mark a live shell exited");
    }

    #[test]
    fn terminal_pane_reader_fd_is_nonblocking() {
        let pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
        let fd = pane.pty.master.as_raw_fd().unwrap();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert!(flags & libc::O_NONBLOCK != 0, "reader fd lacks O_NONBLOCK");
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

    #[test]
    fn terminal_pane_exited_after_shell_exit_command() {
        // Send `exit\n`, pump with deadline until exited; assert status and
        // that write_input returns 0 (broken pipe → exited guard fires).
        let mut pane = TerminalPane::new(24, 80, PathBuf::from("/tmp")).unwrap();
        pane.write_input(b"exit\n");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            pane.pump_output();
            if pane.exited || std::time::Instant::now() >= deadline { break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(pane.exited, "pane must be marked exited after shell exits");
        assert!(pane.status_line().contains("shell exited"),
            "status_line must say 'shell exited' after exit");
        // Confirm exited flag stays true regardless of further pump calls
        pane.pump_output();
        assert!(pane.exited, "exited must stay true after extra pump");
    }

    // Key encoder tests
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn encode_key_ascii_char_returns_byte() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![b'a']);
    }

    #[test]
    fn encode_key_arrow_up_csi_mode() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[A".to_vec());
    }

    #[test]
    fn encode_key_arrow_up_ss3_mode() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let encoded = encode_key(key, true);
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
        assert!(encoded == vec![127] || encoded == vec![8]);
    }

    #[test]
    fn encode_key_down_arrow() {
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[B".to_vec());
    }

    #[test]
    fn encode_key_right_arrow() {
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[C".to_vec());
    }

    #[test]
    fn encode_key_left_arrow() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[D".to_vec());
    }

    #[test]
    fn encode_key_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![3]); // Ctrl-C = 0x03
    }

    #[test]
    fn encode_key_ctrl_a() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![1]); // Ctrl-A = 0x01
    }

    #[test]
    fn encode_key_escape() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![0x1b]);
    }

    #[test]
    fn encode_key_home_csi_mode() {
        let key = KeyEvent::new(KeyCode::Home, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[H".to_vec());
    }

    #[test]
    fn encode_key_end_csi_mode() {
        let key = KeyEvent::new(KeyCode::End, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[F".to_vec());
    }

    #[test]
    fn encode_key_page_up() {
        let key = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[5~".to_vec());
    }

    #[test]
    fn encode_key_page_down() {
        let key = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[6~".to_vec());
    }

    #[test]
    fn encode_key_delete() {
        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[3~".to_vec());
    }

    #[test]
    fn encode_key_backtab() {
        let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1b[Z".to_vec());
    }

    #[test]
    fn encode_key_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, b"\x1bx".to_vec());
    }

    #[test]
    fn encode_key_down_arrow_ss3_mode() {
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let encoded = encode_key(key, true);
        assert_eq!(encoded, b"\x1bOB".to_vec());
    }

    #[test]
    fn encode_key_non_ascii_char_encodes_utf8() {
        // é is U+00E9 → UTF-8: 0xC3 0xA9
        let key = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![0xc3, 0xa9]);
    }

    #[test]
    fn encode_key_alt_non_ascii_char_encodes_utf8() {
        // Alt+é → ESC + UTF-8 bytes of é
        let key = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::ALT);
        let encoded = encode_key(key, false);
        assert_eq!(encoded, vec![0x1b, 0xc3, 0xa9]);
    }
}
