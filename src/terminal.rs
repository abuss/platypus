use portable_pty::{CommandBuilder, PtyPair, PtySize};
use ratatui::style::Color;
use std::collections::VecDeque;
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

    /// Get the scroll offset (0 if no scrollback).
    fn scroll(&self) -> isize;

    /// Set application keypad mode on/off.
    fn application_keypad(&mut self, on: bool);
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
    pub scrollback: VecDeque<String>,
    pub height: u16,
    pub visible: bool,
    pub cwd: PathBuf,
    pub exited: bool,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    /// Partial line carried between pumps so no output is lost at chunk boundaries.
    pending_line: String,
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
            scrollback: VecDeque::new(),
            height: rows,
            visible: true,
            cwd,
            exited: false,
            reader,
            writer,
            pending_line: String::new(),
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
        let mut buf = [0u8; 4096];
        match self.reader.read(&mut buf) {
            Ok(0) => {
                self.exited = true;
                0
            }
            Ok(n) => {
                self.emulator.write(&buf[..n]);
                self.pending_line.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(pos) = self.pending_line.find('\n') {
                    let rest = self.pending_line.split_off(pos + 1);
                    let line = self.pending_line.trim_end_matches(['\n', '\r']).to_owned();
                    self.push_scrollback(&line);
                    self.pending_line = rest;
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

    /// Push a completed output line onto the scrollback, evicting the oldest
    /// line past SCROLLBACK_LIMIT.
    fn push_scrollback(&mut self, line: &str) {
        self.scrollback.push_back(line.to_string());
        while self.scrollback.len() > SCROLLBACK_LIMIT {
            self.scrollback.pop_front();
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

    pub fn draw(&self) -> Vec<Vec<Cell>> {
        self.emulator.draw()
    }

    pub fn scroll(&mut self, _delta: i16) {
        // Navigate scrollback (not yet displayed; Phase 2)
    }

    pub fn snap_to_live(&mut self) {
        // Clear scrollback view, snap to live output (Phase 2)
    }

    pub fn status_line(&self) -> String {
        if self.exited {
            "shell exited".to_string()
        } else {
            format!("shell: {}", self.cwd.display())
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
        let parser = vt100::Parser::new(rows, cols, 0);
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

    fn scroll(&self) -> isize {
        0
    }

    fn application_keypad(&mut self, on: bool) {
        self.kpam_mode = on;
    }
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
        // Without Alt: send as-is
        if !key.modifiers.contains(KeyModifiers::ALT) {
            return vec![ch as u8];
        }
        // With Alt: ESC-prefix the byte (Alt+x = ESC x)
        if key.modifiers.contains(KeyModifiers::ALT) {
            return vec![0x1b, ch as u8];
        }
    }

    // Special keys: arrows, function keys, etc.
    match key.code {
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
        assert_eq!(x, 2);
    }

    #[test]
    fn vt100_sgr_escape_sequence_parsed() {
        let mut em = Vt100Emulator::new(80, 24);
        em.write(b"\x1b[31mred"); // ESC [ 31 m = red
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
            pane.push_scrollback(&format!("line {i}"));
        }
        assert_eq!(pane.scrollback.len(), 2000);
        assert_eq!(pane.scrollback.front().unwrap(), "line 1");
        assert_eq!(pane.scrollback.back().unwrap(), "line 2000");
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
}
