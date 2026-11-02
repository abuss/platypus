use std::path::PathBuf;

use crate::document::Document;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    Edit,
    Action,
}

pub struct App {
    pub docs: Vec<Document>,
    pub active: usize,
    pub mode: Mode,
    /// (message, is_error)
    pub status: Option<(String, bool)>,
    pub quit: bool,
}

impl App {
    pub fn new(paths: Vec<PathBuf>) -> App {
        let mut docs = Vec::new();
        let mut status = None;
        for path in paths {
            match Document::open(&path) {
                Ok(doc) => docs.push(doc),
                Err(e) => status = Some((format!("{}: {e:?}", path.display()), true)),
            }
        }
        if docs.is_empty() {
            docs.push(Document::empty());
        }
        App { docs, active: 0, mode: Mode::Edit, status, quit: false }
    }

    pub fn doc(&self) -> &Document {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }

    pub fn handle_command(&mut self, cmd: crate::keymap::Command) {
        use crate::keymap::Command::*;
        match cmd {
            InsertChar(c) => {
                let d = self.doc_mut();
                d.edit(d.cursor, &c.to_string(), 0);
                d.cursor += 1;
            }
            InsertNewline => {
                let d = self.doc_mut();
                d.edit(d.cursor, "\n", 0);
                d.cursor += 1;
            }
            Indent => {
                let d = self.doc_mut();
                d.edit(d.cursor, "  ", 0);
                d.cursor += 2;
            }
            Backspace => {
                let d = self.doc_mut();
                if d.cursor > 0 {
                    d.edit(d.cursor - 1, "", 1);
                    d.cursor -= 1;
                }
            }
            Delete => {
                let d = self.doc_mut();
                if d.cursor < d.rope.len_chars() {
                    d.edit(d.cursor, "", 1);
                }
            }
            GoToLineStart => {
                let d = self.doc_mut();
                d.cursor = crate::document::line_bounds(&d.rope, d.cursor).0;
            }
            GoToLineEnd => {
                let d = self.doc_mut();
                d.cursor = crate::document::line_bounds(&d.rope, d.cursor).1;
            }
            Move(motion, _select) => {
                // Select::Extend is identical to Move until selections exist (Task 5).
                self.move_cursor(motion);
            }
            EnterActionMode => self.mode = Mode::Action,
            EnterEditMode => self.mode = Mode::Edit,
            Save => match self.doc_mut().save() {
                Ok(()) => self.status = Some(("saved".into(), false)),
                Err(e) => self.status = Some((e, true)),
            },
            Undo => {
                let d = self.doc_mut();
                d.undo();
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            Redo => {
                let d = self.doc_mut();
                d.redo();
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            // Owned by later tasks: Find/Open/CloseBuffer (8/9), Suspend (11).
            Find | Open | CloseBuffer | Suspend => {}
            Quit => self.quit = true,
            Nop => {}
        }
        self.clamp_viewport();
    }

    fn move_cursor(&mut self, motion: crate::keymap::Motion) {
        use crate::keymap::Motion::*;
        let d = self.doc_mut();
        let len = d.rope.len_chars();
        match motion {
            Left => d.cursor = d.cursor.saturating_sub(1),
            Right => d.cursor = (d.cursor + 1).min(len),
            Up | Down => {
                let row = d.rope.char_to_line(d.cursor.min(len));
                let col = d.cursor - d.rope.line_to_char(row);
                // ponytail: column is re-derived per move, no desired-column memory;
                // add it in Phase 0.1 if arrowing through short lines gets annoying.
                let target = match motion {
                    Up if row > 0 => row - 1,
                    Down if row + 1 < d.rope.len_lines() => row + 1,
                    _ => return,
                };
                let (_, end) = crate::document::line_bounds(&d.rope, d.rope.line_to_char(target));
                let target_len = end - d.rope.line_to_char(target);
                d.cursor = d.rope.line_to_char(target) + col.min(target_len);
            }
            PageUp | PageDown => {
                let page = self.viewport_rows().max(1);
                let d = self.doc_mut();
                let row = d.rope.char_to_line(d.cursor.min(len));
                let target = match motion {
                    PageUp => row.saturating_sub(page),
                    PageDown => (row + page).min(d.rope.len_lines().saturating_sub(1)),
                    _ => unreachable!(),
                };
                let start = d.rope.line_to_char(target);
                let (_, end) = crate::document::line_bounds(&d.rope, start);
                let col = (d.cursor - d.rope.line_to_char(row)).min(end - start);
                d.cursor = start + col;
            }
        }
    }

    fn viewport_rows(&self) -> usize {
        crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(24).saturating_sub(1)
    }

    fn clamp_viewport(&mut self) {
        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let view = (h.saturating_sub(1), w);
        let d = self.doc_mut();
        let row = d.rope.char_to_line(d.cursor.min(d.rope.len_chars()));
        let col = d.cursor - d.rope.line_to_char(row);
        (d.scroll_row, d.scroll_col) =
            crate::render::clamp_scroll(row, col, view, (d.scroll_row, d.scroll_col));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Command;

    #[test]
    fn typing_and_undo_via_commands() {
        let mut app = App::new(vec![]);
        for c in ['a', 'b'] {
            app.handle_command(Command::InsertChar(c));
        }
        assert_eq!(app.doc().cursor, 2);
        for c in ['x', 'y', 'z'] {
            app.handle_command(Command::InsertChar(c));
        }
        assert_eq!(app.doc().rope.to_string(), "abxyz");
        for _ in 0..3 {
            app.handle_command(Command::Undo);
        }
        assert_eq!(app.doc().rope.to_string(), "ab");
        assert_eq!(app.doc().cursor, 2);
    }

    #[test]
    fn new_opens_paths_or_empty_buffer() {
        let dir = std::env::temp_dir().join(format!("plat-t3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.txt");
        std::fs::write(&p, "x").unwrap();
        let app = App::new(vec![p.clone()]);
        assert_eq!(app.docs.len(), 1);
        assert_eq!(app.mode, Mode::Edit);
        let empty = App::new(vec![dir.join("missing.txt")]);
        assert_eq!(empty.docs.len(), 1);
        assert!(empty.status.as_ref().unwrap().1, "failed open must set error status");
        std::fs::remove_file(&p).ok();
    }
}
