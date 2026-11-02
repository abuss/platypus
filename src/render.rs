use ratatui::Frame;

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

/// After a terminal size change: recompute every doc's scroll so the cursor
/// stays visible; degenerate sizes clamp to (0,0).
pub fn apply_resize(app: &mut crate::app::App, size: (u16, u16)) {
    let view = (size.1.saturating_sub(1), size.0);
    for doc in app.docs.iter_mut() {
        doc.cursor = doc.cursor.min(doc.rope.len_chars());
        let row = doc.rope.char_to_line(doc.cursor);
        let col = doc.cursor - doc.rope.line_to_char(row);
        (doc.scroll_row, doc.scroll_col) = clamp_scroll(row, col, view, (doc.scroll_row, doc.scroll_col));
    }
}

pub fn draw(f: &mut Frame, app: &App) {    let area = f.area();
    let doc = app.doc();
    let view_rows = area.height.saturating_sub(1) as usize;

    // cursor position (before the buffer borrow below)
    let cur_row = doc.rope.char_to_line(doc.cursor.min(doc.rope.len_chars()));
    if cur_row >= doc.scroll_row && cur_row < doc.scroll_row + view_rows {
        let line_start = doc.rope.line_to_char(cur_row);
        let cur_col = doc.cursor - line_start;
        if cur_col >= doc.scroll_col && cur_col - doc.scroll_col < area.width as usize {
            let _ = f.set_cursor_position(((cur_col - doc.scroll_col) as u16, (cur_row - doc.scroll_row) as u16));
        }
    }

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

    // selection: blue background over the visible part of the range
    if let Some((sel_start, sel_end)) = doc.selection() {
        for row in 0..view_rows {
            let line_idx = doc.scroll_row + row;
            if line_idx >= doc.rope.len_lines() {
                break;
            }
            let line_start = doc.rope.line_to_char(line_idx);
            let line_end = if line_idx + 1 < doc.rope.len_lines() {
                doc.rope.line_to_char(line_idx + 1)
            } else {
                doc.rope.len_chars()
            };
            let from = sel_start.max(line_start);
            let to = sel_end.min(line_end);
            for off in from..to {
                let col = off - line_start;
                if col >= doc.scroll_col && col - doc.scroll_col < area.width as usize {
                    buf[((col - doc.scroll_col) as u16, row as u16)]
                        .set_style(ratatui::style::Style::default().bg(ratatui::style::Color::Blue));
                }
            }
        }
    }

    // live find highlight while the Find prompt is open
    if let Some(pr) = &app.prompt {
        if pr.kind == crate::prompt::PromptKind::Find && !pr.line.is_empty() {
            for row in 0..view_rows {
                let line_idx = doc.scroll_row + row;
                if line_idx >= doc.rope.len_lines() {
                    break;
                }
                let text: String = doc.rope.line(line_idx).chars().take_while(|c| *c != '\n').collect();
                let mut bi = 0;
                while let Some(pos) = text[bi..].find(&pr.line) {
                    let at = bi + pos;
                    let from = text[..at].chars().count();
                    let to = from + pr.line.chars().count();
                    for col in from..to {
                        if col >= doc.scroll_col && col - doc.scroll_col < area.width as usize {
                            buf[((col - doc.scroll_col) as u16, row as u16)].set_style(
                                ratatui::style::Style::default()
                                    .add_modifier(ratatui::style::Modifier::UNDERLINED),
                            );
                        }
                    }
                    bi = at + pr.line.len().max(1);
                }
            }
        }
    }

    // status bar: mode, filename(+), then any status message
    let mut status = if let Some(pr) = &app.prompt {
        use crate::prompt::PromptKind::*;
        match pr.kind {
            Find => format!("find > {}", pr.line),
            Open => format!("open > {}", pr.line),
            SaveAs => format!("save as > {}", pr.line),
            ConfirmQuit => format!("unsaved changes: {} (Tab cycles, Enter confirms, Esc cancels)", pr.choice.label().to_uppercase()),
            ConfirmReload => format!("changed on disk: {} (Tab cycles, Enter confirms, Esc cancels)", pr.choice.label().to_uppercase()),
            Command => format!(":> {}", pr.line),
        }
    } else {
        match app.mode {
            crate::app::Mode::Edit => "EDIT ".to_string(),
            crate::app::Mode::Action => "ACTION ".to_string(),
        }
    };
    if app.prompt.is_none() {
        for (i, d) in app.docs.iter().enumerate() {
            if i > 0 {
                status.push(' ');
            }
            let name = d
                .path
                .as_ref()
                .map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()))
                .unwrap_or_else(|| "[No name]".to_string());
            if i == app.active {
                status.push('[');
                status.push_str(&name);
                if d.dirty {
                    status.push('+');
                }
                status.push(']');
            } else {
                status.push_str(&name);
                if d.dirty {
                    status.push('+');
                }
            }
        }
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

    fn buffer_of(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().buffer().clone()
    }

    fn row_string(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
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
    fn find_prompt_underlines_live_matches() {
        use crate::prompt::{Prompt, PromptKind};
        use ratatui::style::Modifier;
        let dir = std::env::temp_dir().join(format!("plat-t8f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("find.txt");
        std::fs::write(&p, "hello ell\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.prompt = Some(Prompt::new(PromptKind::Find, "ell".into()));
        let buf = buffer_of(&app, 80, 24);
        assert!(buf[(1, 0)].modifier.contains(Modifier::UNDERLINED), "match 'ell' at 1 underlined");
        assert!(buf[(6, 0)].modifier.contains(Modifier::UNDERLINED), "match at 6 underlined");
        assert!(!buf[(0, 0)].modifier.contains(Modifier::UNDERLINED), "'h' not underlined");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn selection_painted_with_blue_bg() {
        use crate::document::Selection;
        use ratatui::style::Color;
        let dir = std::env::temp_dir().join(format!("plat-t5s-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sel.txt");
        std::fs::write(&p, "hello\n").unwrap();
        let mut app = App::new(vec![p.clone()]);
        app.doc_mut().selections.push(Selection { anchor: 1, head: 3 });
        let buf = buffer_of(&app, 80, 24);
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
        assert_eq!(buf[(1, 0)].bg, Color::Blue, "'e' selected");
        assert_eq!(buf[(2, 0)].bg, Color::Blue, "'l' selected");
        assert_eq!(buf[(3, 0)].bg, Color::Reset);
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
        assert_eq!(*buf.area(), ratatui::layout::Rect::new(0, 0, 1, 1));
        let _ = buf[(0, 0)].symbol();
        assert_eq!(app.mode, Mode::Edit);
    }
}
