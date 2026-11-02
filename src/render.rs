use ratatui::{buffer::Buffer, layout::Rect, Frame};

use crate::app::App;

/// Keep cursor visible in a no-wrap viewport; returns new (scroll_row, scroll_col).
/// `area` is the viewport area (rows, cols), `scroll` the current scroll.
pub fn clamp_scroll(
    cursor_row: usize,
    cursor_col: usize,
    area: (u16, u16),
    scroll: (usize, usize),
) -> (usize, usize) {
    let rows = area.0 as usize;
    let cols = area.1 as usize;
    let (mut sr, mut sc) = scroll;
    if rows == 0 {
        sr = 0;
    } else if cursor_row < sr {
        sr = cursor_row;
    } else if cursor_row >= sr + rows {
        sr = cursor_row + 1 - rows;
    }
    if cols == 0 {
        sc = 0;
    } else if cursor_col < sc {
        sc = cursor_col;
    } else if cursor_col >= sc + cols {
        sc = cursor_col + 1 - cols;
    }
    (sr, sc)
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let doc = app.doc();
    let view_rows = area.height.saturating_sub(1) as usize;
    let buf = f.buffer_mut();

    for row in 0..view_rows {
        let line_idx = doc.scroll_row + row;
        if line_idx >= doc.rope.len_lines() {
            break;
        }
        let line = doc.rope.line(line_idx);
        // ropey's line() includes the trailing '\n' — filter it (also guards
        // any other control char from cell_width)
        let visible = line.chars().take_while(|c| *c != '\n');
        for (col, ch) in visible.skip(doc.scroll_col).take(area.width as usize).enumerate() {
            buf[(col as u16, row as u16)].set_char(ch);
        }
    }

    // status bar: mode, filename(+), then any status message
    let mut status = match app.mode {
        crate::app::Mode::Edit => "EDIT ".to_string(),
        crate::app::Mode::Action => "ACTION ".to_string(),
    };
    match &doc.path {
        Some(p) => {
            status.push_str(&p.display().to_string());
            if doc.dirty {
                status.push('+');
            }
        }
        None => status.push_str("[No name]"),
    }
    if let Some((msg, _)) = &app.status {
        status.push_str("  ");
        status.push_str(msg);
    }
    let status_y = view_rows as u16;
    if status_y < area.height {
        for (x, ch) in status.chars().take_while(|c| !c.is_control()).take(area.width as usize).enumerate() {
            buf[(x as u16, status_y)].set_char(ch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode};
    use ratatui::backend::TestBackend;

    fn buffer_of(app: &App, w: u16, h: u16) -> Buffer {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    fn row_string(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect()
    }

    #[test]
    fn renders_visible_lines_and_status_bar() {
        let dir = std::env::temp_dir().join(format!("plat-t3r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("file.txt");
        std::fs::write(&p, "alpha\nbeta\ngamma\n").unwrap();
        let app = App::new(vec![p.clone()]);
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(row_string(&buf, 0, 80).trim_end(), "alpha");
        assert_eq!(row_string(&buf, 1, 80).trim_end(), "beta");
        assert_eq!(row_string(&buf, 2, 80).trim_end(), "gamma");
        let status = row_string(&buf, 23, 80);
        assert!(status.contains("EDIT"), "status must show mode: {status:?}");
        assert!(status.contains("file.txt"), "status must show path: {status:?}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn clamp_scroll_follows_cursor_down() {
        // cursor row 30 in a 20-row window starting at scroll 0 -> window ends at row 30
        assert_eq!(clamp_scroll(30, 0, (20, 80), (0, 0)), (11, 0));
        // cursor already visible: no change
        assert_eq!(clamp_scroll(5, 0, (20, 80), (0, 0)), (0, 0));
    }

    #[test]
    fn clamp_scroll_follows_cursor_right() {
        assert_eq!(clamp_scroll(0, 95, (20, 80), (0, 0)), (0, 16));
    }

    #[test]
    fn empty_doc_one_column_terminal_renders_without_panic() {
        let app = App::new(vec![]);
        let buf = buffer_of(&app, 1, 1);
        assert_eq!(*buf.area(), Rect::new(0, 0, 1, 1));
        let _ = buf[(0, 0)].symbol();
        assert_eq!(app.mode, Mode::Edit);
    }
}
