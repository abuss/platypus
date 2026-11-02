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
    pub prompt: Option<crate::prompt::Prompt>,
    pub search_history: Vec<String>,
    /// set when a ConfirmQuit prompt was raised by Ctrl-Q (quit) vs Ctrl-W (close)
    pub quit_after_confirm: bool,
    pub mtime_cache: std::collections::HashMap<std::path::PathBuf, std::time::SystemTime>,
    pub reload_pending: Option<usize>,
    pub suspend_requested: bool,
    /// PLAT_KEYDEBUG=1: show every raw KeyEvent in the status bar (field triage for terminals)
    pub key_debug: bool,
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
        App { docs, active: 0, mode: Mode::Edit, status, quit: false, pending: None, clipboard: String::new(), prompt: None, search_history: Vec::new(), quit_after_confirm: false, mtime_cache: std::collections::HashMap::new(), reload_pending: None, suspend_requested: false, key_debug: std::env::var_os("PLAT_KEYDEBUG").is_some() }
    }

    pub fn doc(&self) -> &Document {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }

    pub fn handle_command(&mut self, cmd: crate::keymap::Command) {
        use crate::keymap::Command::*;
        if self.prompt.is_some() {
            // the prompt owns the keyboard while open
            match cmd {
                PromptChar(c) => self.prompt.as_mut().unwrap().line.push(c),
                PromptBackspace => {
                    self.prompt.as_mut().unwrap().line.pop();
                }
                PromptHistoryUp => self.prompt.as_mut().unwrap().history_up(),
                PromptHistoryDown => {}
                PromptCycle => {
                    let p = self.prompt.as_mut().unwrap();
                    p.choice = p.choice.cycle();
                }
                PromptCancel => match self.prompt.as_ref().map(|p| p.kind) {
                    // confirm prompts must run their Cancel branch (rearm etc.)
                    Some(crate::prompt::PromptKind::ConfirmReload)
                    | Some(crate::prompt::PromptKind::ConfirmQuit) => self.finish_prompt(),
                    _ => self.prompt = None,
                },
                PromptSubmit => self.finish_prompt(),
                _ => {}
            }
            return;
        }
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
                if let Some(inv) = d.history.undo(&mut d.rope) {
                    // park the cursor at the undone change, not where it happened to be
                    d.cursor = inv.at.min(d.rope.len_chars());
                }
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            Redo => {
                let d = self.doc_mut();
                if let Some(c) = d.history.redo(&mut d.rope) {
                    d.cursor = (c.at + c.inserted.chars().count()).min(d.rope.len_chars());
                }
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            Find => {
                let prefill = self
                    .doc()
                    .selection()
                    .and_then(|(a, b)| if b > a { Some(self.doc().slice(a, b - a)) } else { None })
                    .or_else(|| self.doc().search.clone())
                    .unwrap_or_default();
                let mut p = crate::prompt::Prompt::new(crate::prompt::PromptKind::Find, prefill);
                p.history = self.search_history.clone();
                self.prompt = Some(p);
            }
            Open => {
                let p = crate::prompt::Prompt::new(crate::prompt::PromptKind::Open, String::new());
                self.prompt = Some(p);
            }
            FindNext => {
                if !self.doc_mut().search_next(true) {
                    let t = self.doc().search.clone().unwrap_or_default();
                    self.status = Some((format!("no matches for '{t}'"), true));
                }
            }
            FindPrev => {
                if !self.doc_mut().search_next(false) {
                    let t = self.doc().search.clone().unwrap_or_default();
                    self.status = Some((format!("no matches for '{t}'"), true));
                }
            }
            // prompt commands without an open prompt: no-op (routing above owns them)
            PromptChar(_) | PromptBackspace | PromptHistoryUp | PromptHistoryDown
            | PromptSubmit | PromptCancel | PromptCycle => {}
            CloseBuffer => self.request_close(),
            CommandPrefix => {
                self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::Command, String::new()));
            }
            Suspend => self.suspend_requested = true, // main loop performs restore+SIGTSTP
            Quit => {
                if self.docs.iter().any(|d| d.dirty) {
                    self.quit_after_confirm = true;
                    self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmQuit, String::new()));
                } else {
                    self.quit = true;
                }
            }
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

    fn finish_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        use crate::prompt::PromptKind::*;
        match p.kind {
            Find => {
                if !p.line.is_empty() {
                    if self.search_history.last().map(|h| *h != p.line).unwrap_or(true) {
                        self.search_history.push(p.line.clone());
                    }
                    let term = p.line;
                    let d = self.doc_mut();
                    d.set_search(&term);
                    if d.search_matches.is_empty() {
                        self.status = Some((format!("no matches for '{term}'"), true));
                    } else {
                        d.search_next(true);
                    }
                }
            }
            Open => self.open_path(&p.line),
            SaveAs => {
                if let Err(e) = self.doc_mut().save_as(std::path::Path::new(&p.line)) {
                    self.status = Some((e, true));
                }
            }
            Command => {
                let line = p.line.trim().to_string();
                if line.is_empty() {
                } else if line == "q" {
                    self.request_close();
                } else if line == "w" {
                    if let Err(e) = self.doc_mut().save() {
                        self.status = Some((e, true));
                    }
                } else if line == "wq" {
                    if let Err(e) = self.doc_mut().save() {
                        self.status = Some((e, true));
                    } else {
                        self.close_active();
                    }
                } else if let Some(rest) = line.strip_prefix("w ") {
                    if let Err(e) = self.doc_mut().save_as(std::path::Path::new(rest.trim())) {
                        self.status = Some((e, true));
                    }
                } else if let Some(rest) = line.strip_prefix("e ") {
                    self.open_path(rest.trim());
                } else {
                    self.status = Some((format!("unknown command: {line}"), true));
                }
            }
            ConfirmQuit => {
                use crate::prompt::Choice::*;
                match p.choice {
                    Cancel => {}
                    Discard => self.after_confirm_close(),
                    Save => {
                        if let Err(e) = self.doc_mut().save() {
                            self.status = Some((e, true));
                        } else {
                            self.after_confirm_close();
                        }
                    }
                }
            }
            ConfirmReload => {
                use crate::prompt::Choice::*;
                let idx = self.reload_pending.take();
                match (p.choice, idx) {
                    (Cancel, Some(idx)) => {
                        // rearm: sentinel mtime makes the next poll flag this path again
                        if let Some(path) = self.docs[idx].path.clone() {
                            self.mtime_cache.insert(path, std::time::SystemTime::UNIX_EPOCH);
                        }
                    }
                    (Save, Some(idx)) => {
                        // "keep mine": rebaseline to the disk mtime, buffer untouched
                        if let Some(path) = self.docs[idx].path.clone() {
                            if let Some(m) = crate::watch::mtime(&path) {
                                self.mtime_cache.insert(path, m);
                            }
                        }
                    }
                    (Discard, Some(idx)) => self.reload_doc(idx),
                    (_, None) => {}
                }
            }
        }
    }

    fn open_path(&mut self, path: &str) {
        let path = std::path::PathBuf::from(path);
        match crate::document::Document::open(&path) {
            Ok(d) => {
                self.docs.push(d);
                self.active = self.docs.len() - 1;
                self.mode = Mode::Edit;
            }
            Err(e) => self.status = Some((format!("{}: {e:?}", path.display()), true)),
        }
    }

    fn request_close(&mut self) {
        if self.doc().dirty {
            self.quit_after_confirm = false;
            self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmQuit, String::new()));
        } else {
            self.close_active();
        }
    }

    fn after_confirm_close(&mut self) {
        if self.quit_after_confirm {
            self.quit = true;
        } else {
            self.close_active();
        }
    }

    /// React to an externally-modified file: clean → silent reload;
    /// dirty → ConfirmReload prompt (keep mine / reload / cancel+rearm).
    pub fn handle_external_change(&mut self, path: &std::path::Path) {
        let Some(idx) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) else { return };
        if self.docs[idx].dirty {
            self.reload_pending = Some(idx);
            self.active = idx;
            self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmReload, String::new()));
        } else {
            self.reload_doc(idx);
        }
    }

    fn reload_doc(&mut self, idx: usize) {
        let Some(path) = self.docs[idx].path.clone() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => {
                    let line_ending = if text.contains("\r\n") {
                        crate::document::LineEnding::CRLF
                    } else {
                        crate::document::LineEnding::LF
                    };
                    let normalized = text.replace("\r\n", "\n");
                    let d = &mut self.docs[idx];
                    // the reload itself is one undoable Change: undo restores
                    // the pre-external-edit buffer (history survives reload)
                    let old = d.rope.to_string();
                    let change = crate::undo::Change { at: 0, inserted: normalized, removed: old };
                    change.apply(&mut d.rope);
                    d.history.push(change);
                    d.dirty = false;
                    d.line_ending = line_ending;
                    d.cursor = d.cursor.min(d.rope.len_chars());
                    d.selections.clear();
                    if let Some(m) = crate::watch::mtime(&path) {
                        self.mtime_cache.insert(path, m);
                    }
                    self.clamp_viewport();
                }
                Err(_) => self.status = Some((format!("{}: not valid UTF-8", path.display()), true)),
            },
            Err(e) => self.status = Some((format!("{}: {e}", path.display()), true)),
        }
    }

    fn close_active(&mut self) {
        if self.docs.len() == 1 {
            self.quit = true;
            return;
        }
        self.docs.remove(self.active);
        self.active = self.active.min(self.docs.len() - 1);
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

    /// Bracketed-paste event: insert the pasted text at the cursor verbatim
    /// (newlines normalized to LF), as ONE undoable change.
    pub fn handle_paste(&mut self, text: &str) {
        let norm = text.replace("\r\n", "\n").replace('\r', "\n");
        if norm.is_empty() {
            return;
        }
        let d = self.doc_mut();
        let at = d.cursor;
        d.edit(at, &norm, 0);
        d.cursor = at + norm.chars().count();
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
    fn confirm_save_with_failing_write_aborts_close() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("plat-t9f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("ro.txt");
        std::fs::write(&a, "disk").unwrap();
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o444)).unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::CloseBuffer);
        app.handle_command(Command::PromptCycle); // Cancel -> Save
        app.handle_command(Command::PromptSubmit); // Save fails (EACCES)
        assert!(!app.quit, "failed save must abort the close");
        assert_eq!(app.docs.len(), 1);
        assert!(app.doc().dirty, "buffer still dirty with content intact");
        assert_eq!(app.doc().rope.to_string(), "Xdisk");
        assert!(app.status.as_ref().unwrap().1, "error shown");
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn clean_doc_auto_reloads_and_keeps_undo_history() {
        use ropey::Rope;
        let dir = std::env::temp_dir().join(format!("plat-t10c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("ext.txt");
        std::fs::write(&a, "one").unwrap();
        let mut app = App::new(vec![a.clone()]);
        // local edit + save
        app.doc_mut().cursor = app.doc().rope.len_chars();
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::Save);
        // external change (sleep so mtime definitely moves)
        std::fs::write(&a, "TWO").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut cache = std::collections::HashMap::new();
        crate::watch::changed_paths(&app.docs, &mut cache); // baseline
        std::fs::write(&a, "THREE").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let changed = crate::watch::changed_paths(&app.docs, &mut cache);
        assert_eq!(changed, vec![a.clone()]);
        app.mtime_cache = cache;
        app.handle_external_change(&a);
        assert_eq!(app.doc().rope.to_string(), "THREE");
        assert!(app.prompt.is_none(), "clean reload is silent");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "oneX", "undo survives reload of same path");
        std::fs::remove_file(&a).ok();
        let _ = Rope::from_str("");
    }

    #[test]
    fn dirty_doc_gets_confirm_prompt_and_rearms_on_cancel() {
        let dir = std::env::temp_dir().join(format!("plat-t10d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("dirty10.txt");
        std::fs::write(&a, "disk").unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('L')); // dirty, unsaved
        std::fs::write(&a, "external").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut cache = std::collections::HashMap::new();
        crate::watch::changed_paths(&app.docs, &mut cache);
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&a, "external2").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let changed = crate::watch::changed_paths(&app.docs, &mut cache);
        assert_eq!(changed, vec![a.clone()]);
        app.mtime_cache = cache;
        app.handle_external_change(&a);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmReload);
        app.handle_command(Command::PromptCancel);
        assert_eq!(app.doc().rope.to_string(), "Ldisk", "cancel: buffer untouched");
        assert!(app.doc().dirty);
        // rearm: watcher must flag the same path again
        let again = crate::watch::changed_paths(&app.docs, &mut app.mtime_cache);
        assert_eq!(again, vec![a.clone()], "cancel re-arms the watcher");
        // discard -> reload
        app.handle_external_change(&a);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptSubmit); // Discard
        assert_eq!(app.doc().rope.to_string(), "external2");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn tab_bar_shows_docs_bracketing_active() {
        let dir = std::env::temp_dir().join(format!("plat-t9-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "1").unwrap();
        std::fs::write(&b, "2").unwrap();
        let app = App::new(vec![a.clone(), b.clone()]);
        assert_eq!(app.docs.len(), 2);
        // render it and check status row
        use ratatui::backend::TestBackend;
        let mut term = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| crate::render::draw(f, &app)).unwrap();
        let buf = term.backend().buffer();
        let row: String = (0..80).map(|x| buf[(x, 23)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row.contains("[a.txt"), "active doc bracketed: {row:?}");
        assert!(row.contains("b.txt"), "other doc listed: {row:?}");
        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }

    #[test]
    fn close_dirty_asks_then_discard_or_cancel() {
        let dir = std::env::temp_dir().join(format!("plat-t9c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("dirty.txt");
        std::fs::write(&a, "disk").unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::CloseBuffer);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmQuit);
        app.handle_command(Command::PromptCancel);
        assert_eq!(app.docs.len(), 1, "cancel keeps doc");
        assert!(app.doc().dirty);
        assert!(!app.quit);
        app.handle_command(Command::CloseBuffer);
        app.handle_command(Command::PromptCycle); // Cancel -> Save
        app.handle_command(Command::PromptCycle); // Save -> Discard
        app.handle_command(Command::PromptSubmit);
        assert!(app.quit, "closing the last doc quits");
        assert_eq!(std::fs::read_to_string(&a).unwrap_or_default(), "disk", "file on disk unchanged by discard-close");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn colon_w_q_wq_e_commands() {
        let dir = std::env::temp_dir().join(format!("plat-t9x-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs_new = dir.join("new.txt");
        let abs_other = dir.join("other.txt");
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('v'));
        app.handle_command(Command::CommandPrefix);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::Command);
        for c in format!("w {}", abs_new.display()).chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(std::fs::read_to_string(&abs_new).ok(), Some("v".into()));
        assert_eq!(app.doc().path.as_deref(), Some(abs_new.as_path()));
        assert!(!app.doc().dirty);
        // :e opens another buffer
        std::fs::write(&abs_other, "O").unwrap();
        app.handle_command(Command::CommandPrefix);
        for c in format!("e {}", abs_other.display()).chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.docs.len(), 2);
        assert_eq!(app.doc().rope.to_string(), "O");
        // :q on clean doc closes it
        app.handle_command(Command::CommandPrefix);
        for c in "q".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.docs.len(), 1);
        assert!(!app.quit);
        // :wq on clean last doc saves and quits
        app.handle_command(Command::CommandPrefix);
        for c in "wq".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert!(app.quit);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_q_dirty_prompts_before_quitting() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('x'));
        app.handle_command(Command::Quit);
        assert!(!app.quit, "dirty buffer must be confirmed first");
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmQuit);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptSubmit); // discard
        assert!(app.quit);
    }

    #[test]
    fn resize_reclamps_scroll_without_panic() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str(&"line\n".repeat(40));
        app.doc_mut().cursor = "line\n".repeat(30).chars().count(); // row 30
        crate::render::apply_resize(&mut app, (80, 10));
        assert!(app.doc().scroll_row + 9 >= 30, "cursor row visible in 10-row frame");
        assert!(app.doc().scroll_row <= 30);
        crate::render::apply_resize(&mut app, (1, 1)); // degenerate: no panic, clamped
        assert_eq!(app.doc().scroll_row, 0);
        assert_eq!(app.doc().scroll_col, 0);
    }

    #[test]
    fn suspend_requests_shell_suspension_via_flag() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::Suspend);
        assert!(app.suspend_requested, "main loop performs the actual SIGTSTP");
    }

    #[test]
    fn find_wraps_and_reports_no_matches() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("TARGET up here\nrest of the file\n");
        app.doc_mut().cursor = 20; // below the only match
        app.handle_command(Command::Find);
        for c in "TARGET".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.doc().selection(), Some((0, 6)), "find from below the match must wrap to it");
        app.doc_mut().selections.clear();
        app.handle_command(Command::Find); // prefilled "TARGET"
        for _ in 0..6 {
            app.handle_command(Command::PromptBackspace);
        }
        for c in "zzz".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.doc().selection(), None);
        let (msg, err) = app.status.clone().unwrap();
        assert!(err && msg.contains("no matches"), "user must be told, not left guessing: {msg:?}");
    }

    #[test]
    fn find_prompt_flow() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("hello ell");
        app.doc_mut().cursor = 0;
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::Find);
        for c in "ell".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell");
        app.handle_command(Command::PromptSubmit);
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().selection(), Some((1, 4)), "first match selected");
        app.handle_command(Command::FindNext);
        assert_eq!(app.doc().selection(), Some((6, 9)), "n advances");
        app.handle_command(Command::FindPrev);
        assert_eq!(app.doc().selection(), Some((1, 4)), "N goes back");
        // prefill from active selection
        app.doc_mut().selections.clear();
        app.doc_mut().selections.push(crate::document::Selection { anchor: 6, head: 9 });
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell", "prefilled from selection");
        app.handle_command(Command::PromptCancel);
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().selection(), Some((6, 9)), "cancel leaves state untouched");
        // reopen with no selection: prefills from last search term
        app.doc_mut().selections.clear();
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell", "prefill from last search");
        app.handle_command(Command::PromptCancel);
    }

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
    fn bracketed_paste_inserts_literally_as_one_undo_step() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("AB\n");
        app.doc_mut().cursor = 0;
        app.handle_paste("one\ntwo\tthree\r\n");
        {
            let d = app.doc();
            assert_eq!(d.rope.to_string(), "one\ntwo\tthree\nAB\n", "CRLF normalizes, Tab stays literal");
            assert_eq!(d.cursor, "one\ntwo\tthree\n".chars().count(), "cursor lands at end of pasted text");
        }
        assert!(app.doc_mut().undo(), "the whole paste is a single undo step");
        assert_eq!(app.doc().rope.to_string(), "AB\n");
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
    fn reload_updates_line_ending_and_saves_back_as_crlf() {
        let dir = std::env::temp_dir().join(format!("plat-fr1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("le.txt");
        std::fs::write(&a, "a\nb\n").unwrap();
        let mut app = App::new(vec![a.clone()]);
        assert_eq!(app.doc().line_ending, crate::document::LineEnding::LF);
        // external tool rewrites the file with CRLF
        std::fs::write(&a, "a\r\nb\r\n").unwrap();
        app.handle_external_change(&a); // clean -> silent reload
        assert_eq!(app.doc().rope.to_string(), "a\nb\n");
        assert_eq!(app.doc().line_ending, crate::document::LineEnding::CRLF, "reload must re-detect line endings");
        app.handle_command(Command::GoToLineStart);
        app.handle_command(Command::InsertChar('Z'));
        app.handle_command(Command::Save);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "Za\r\nb\r\n", "save must use the reloaded line endings");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn reload_clamps_viewport_to_new_content() {
        use ropey::Rope;
        let dir = std::env::temp_dir().join(format!("plat-fr2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("shrink.txt");
        std::fs::write(&a, "line\n".repeat(40)).unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.doc_mut().rope = Rope::from_str(&"line\n".repeat(40));
        app.doc_mut().cursor = "line\n".repeat(30).chars().count(); // row 30
        app.doc_mut().scroll_row = 25;
        std::fs::write(&a, "tiny\nfile\n").unwrap();
        app.handle_external_change(&a); // clean reload
        let cur_row = app.doc().rope.char_to_line(app.doc().cursor.min(app.doc().rope.len_chars()));
        assert!(app.doc().scroll_row <= cur_row, "scroll must not sit past the new end of file");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn undo_redo_move_cursor_to_the_change() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("hello");
        app.handle_command(Command::GoToLineStart);
        app.handle_command(Command::InsertChar('X')); // change at offset 0
        for _ in 0..6 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Move));
        }
        assert_eq!(app.doc().cursor, 6);
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().cursor, 0, "undo jumps the cursor to the undone change");
        app.handle_command(Command::Redo);
        assert_eq!(app.doc().cursor, 1, "redo parks the cursor at the end of the redone text");
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
