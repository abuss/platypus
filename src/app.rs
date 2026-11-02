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
    pub pending: Option<crate::keymap::Pending>,
    pub clipboard: String,
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
        App { docs, active: 0, mode: Mode::Edit, status, quit: false, pending: None, clipboard: String::new() }
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
            Move(motion, select) => {
                let pre = self.doc().cursor;
                self.move_cursor(motion);
                let d = self.doc_mut();
                match select {
                    crate::keymap::Select::Move => d.selections.clear(),
                    crate::keymap::Select::Extend => {
                        let c = d.cursor;
                        match d.selections.last_mut() {
                            Some(sel) => sel.head = c,
                            None => d.selections.push(crate::document::Selection { anchor: pre, head: c }),
                        }
                    }
                }
            }
            BeginSelectChar => {
                let d = self.doc_mut();
                if d.selections.last().and_then(|s| s.range()).is_some() {
                    d.selections.clear();
                } else if d.cursor < d.rope.len_chars() {
                    d.selections.push(crate::document::Selection { anchor: d.cursor, head: d.cursor + 1 });
                }
            }
            SelectLine => {
                let d = self.doc_mut();
                let (start, _) = crate::document::line_bounds(&d.rope, d.cursor);
                let line = d.rope.char_to_line(start);
                let end = if line + 1 >= d.rope.len_lines() {
                    d.rope.len_chars()
                } else {
                    d.rope.line_to_char(line + 1)
                };
                d.selections.clear();
                d.selections.push(crate::document::Selection { anchor: start, head: end });
            }
            ExpandLine => {
                let d = self.doc_mut();
                let head_cursor = d.cursor;
                let (start, _) = crate::document::line_bounds(&d.rope, head_cursor);
                let line = d.rope.char_to_line(start);
                let end = if line + 1 >= d.rope.len_lines() {
                    d.rope.len_chars()
                } else {
                    d.rope.line_to_char(line + 1)
                };
                match d.selections.last_mut() {
                    Some(sel) => {
                        sel.anchor = sel.anchor.min(start);
                        sel.head = sel.head.max(end);
                    }
                    None => d.selections.push(crate::document::Selection { anchor: start, head: end }),
                }
            }
            EnterActionMode => self.mode = Mode::Action,
            EnterEditMode => {
                if self.pending.take().is_some() {
                    return; // Esc cancels a pending key capture
                }
                // Esc in Action mode: first press cancels an active selection.
                if self.mode == Mode::Action && self.doc().selection().is_some() {
                    self.doc_mut().selections.clear();
                } else {
                    self.mode = Mode::Edit;
                }
            }
            JumpCharCapture { backward } => {
                self.pending = Some(if backward {
                    crate::keymap::Pending::JumpBackward
                } else {
                    crate::keymap::Pending::JumpForward
                });
            }
            JumpToChar { ch, backward } => {
                self.pending = None;
                let d = self.doc_mut();
                if let Some(target) = crate::document::jump_char(&d.rope, d.cursor, ch, backward) {
                    d.cursor = target;
                }
            }
            ObjectPrefix => self.pending = Some(crate::keymap::Pending::Object),
            Yank => {
                if let Some((a, b)) = self.doc().selection() {
                    self.clipboard = self.doc().slice(a, b - a);
                }
            }
            PasteAfter => self.paste(true),
            PasteBefore => self.paste(false),
            DeleteSelection => {
                if let Some((a, b)) = self.doc().selection() {
                    // d copies too: yank-and-delete, one fewer key by design
                    let text = self.doc().slice(a, b - a);
                    {
                        let d = self.doc_mut();
                        d.edit(a, "", b - a);
                        d.cursor = a;
                        d.selections.clear();
                    }
                    self.clipboard = text;
                }
            }
            Change => {
                if let Some((a, b)) = self.doc().selection() {
                    let d = self.doc_mut();
                    d.edit(a, "", b - a);
                    d.cursor = a;
                    d.selections.clear();
                }
                self.mode = Mode::Edit;
            }
            SelectObject(c) => {
                self.pending = None;
                let d = self.doc_mut();
                if let Some((a, h)) = crate::document::object_range(&d.rope, d.cursor, c) {
                    d.selections.clear();
                    d.selections.push(crate::document::Selection { anchor: a, head: h });
                }
            }
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
            WordForward => d.cursor = crate::document::word_next(&d.rope, d.cursor),
            WordBackward => d.cursor = crate::document::word_prev(&d.rope, d.cursor),
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

    fn paste(&mut self, after: bool) {
        let text = self.clipboard.clone();
        if text.is_empty() {
            return;
        }
        let d = self.doc_mut();
        let at = match d.selection() {
            Some((_, b)) if after => b,
            Some((a, _)) => a,
            None => {
                if after {
                    (d.cursor + 1).min(d.rope.len_chars())
                } else {
                    d.cursor
                }
            }
        };
        d.edit(at, &text, 0);
        d.cursor = at + text.chars().count();
        d.selections.clear();
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
    fn yank_paste_sequence() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 0;
        for _ in 0..3 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Extend));
        }
        app.handle_command(Command::Yank); // clipboard = "foo"
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('(')); // selects (4,7) "bar"
        app.handle_command(Command::PasteAfter);
        assert_eq!(app.doc().rope.to_string(), "foo(barfoo)", "paste after selection end");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "foo(bar)");
    }

    #[test]
    fn empty_clipboard_paste_is_noop() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::PasteAfter);
        assert_eq!(app.doc().rope.to_string(), "a");
    }

    #[test]
    fn change_deletes_selection_enters_edit_single_undoable() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('('));
        app.handle_command(Command::Change);
        assert_eq!(app.doc().rope.to_string(), "foo()");
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.doc().cursor, 4);
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "foo(bar)");
    }

    #[test]
    fn delete_selection_and_paste_before() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 0;
        for _ in 0..3 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Extend));
        }
        app.handle_command(Command::Yank); // clipboard = "foo"
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('(')); // (4,7) "bar"
        app.handle_command(Command::PasteBefore);
        assert_eq!(app.doc().rope.to_string(), "foo(foobar)", "paste before selection start");
        app.handle_command(Command::Undo);
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('('));
        app.handle_command(Command::DeleteSelection);
        assert_eq!(app.doc().rope.to_string(), "foo()");
        assert_eq!(app.doc().selection(), None);
    }

    #[test]
    fn word_jumps_via_commands() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 4; // at 'x'
        app.handle_command(Command::Move(crate::keymap::Motion::WordForward, crate::keymap::Select::Move));
        assert_eq!(app.doc().cursor, 6, "w from 'x' lands at start of '='");
        app.handle_command(Command::Move(crate::keymap::Motion::WordBackward, crate::keymap::Select::Move));
        assert_eq!(app.doc().cursor, 4, "b returns to 'x'");
    }

    #[test]
    fn f_jump_to_char_and_miss_no_move() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 4;
        app.handle_command(Command::JumpToChar { ch: ')', backward: false });
        assert_eq!(app.doc().cursor, 20, "f ) lands on ')'");
        app.handle_command(Command::JumpToChar { ch: 'Z', backward: false });
        assert_eq!(app.doc().cursor, 20, "no match: cursor stays");
    }

    #[test]
    fn textobjects_select_inside() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 13; // inside parens
        app.handle_command(Command::SelectObject('('));
        assert_eq!(app.doc().selection(), Some((12, 20)), "inside parens selects 'bar, baz'");

        let mut app2 = App::new(vec![]);
        app2.doc_mut().rope = Rope::from_str("say \"hi now\"");
        app2.doc_mut().cursor = 9; // inside quotes
        app2.handle_command(Command::SelectObject('"'));
        assert_eq!(app2.doc().selection(), Some((5, 11)), "inside quotes selects 'hi now'");

        let mut app3 = App::new(vec![]);
        app3.doc_mut().rope = Rope::from_str("abc");
        app3.doc_mut().cursor = 1;
        app3.handle_command(Command::SelectObject('"'));
        assert_eq!(app3.doc().selection(), None, "unmatched quote: unchanged");
    }

    #[test]
    fn shift_motion_extends_selection() {
        let mut app = App::new(vec![]);
        for c in ['h', 'e', 'l', 'l', 'o'] {
            app.handle_command(Command::InsertChar(c));
        }
        app.handle_command(Command::Move(crate::keymap::Motion::Left, crate::keymap::Select::Extend));
        app.handle_command(Command::Move(crate::keymap::Motion::Left, crate::keymap::Select::Extend));
        assert_eq!(app.doc().selection(), Some((3, 5)), "anchor stays at 5 while head extends left");
        // plain move clears selection
        app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Move));
        assert_eq!(app.doc().selection(), None);
    }

    #[test]
    fn space_toggles_char_selection_under_cursor() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::GoToLineStart);
        app.mode = Mode::Action;
        app.handle_command(Command::BeginSelectChar);
        assert_eq!(app.doc().selection(), Some((0, 1)));
        app.handle_command(Command::BeginSelectChar);
        assert_eq!(app.doc().selection(), None, "second Space cancels");
    }

    #[test]
    fn esc_two_step_cancels_selection_then_edit() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::GoToLineStart);
        app.handle_command(Command::EnterActionMode);
        app.handle_command(Command::SelectLine);
        assert_eq!(app.doc().selection(), Some((0, 1)), "whole line, no trailing newline in doc");
        app.handle_command(Command::EnterEditMode); // Esc with active selection
        assert_eq!(app.doc().selection(), None);
        assert_eq!(app.mode, Mode::Action, "first Esc only cancels the selection");
        app.handle_command(Command::EnterEditMode);
        assert_eq!(app.mode, Mode::Edit);
    }

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
