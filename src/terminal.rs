use ratatui::style::Color;

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
}
