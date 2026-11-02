use std::fs;
use std::path::{Path, PathBuf};

use ropey::Rope;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LineEnding {
    LF,
    CRLF,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    Io(String),
    NotUtf8,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// Normalized (min, max) range; None when empty.
    pub fn range(&self) -> Option<(usize, usize)> {
        if self.head == self.anchor {
            None
        } else {
            Some((self.anchor.min(self.head), self.anchor.max(self.head)))
        }
    }
}

pub struct Document {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub line_ending: LineEnding,
    pub history: crate::undo::History,
    pub scroll_row: usize,
    pub scroll_col: usize,
    pub cursor: usize,
    pub selections: Vec<Selection>,
}

impl Document {
    pub fn empty() -> Document {
        Document {
            rope: Rope::new(),
            path: None,
            dirty: false,
            line_ending: LineEnding::LF,
            history: crate::undo::History::new(),
            scroll_row: 0,
            scroll_col: 0,
            cursor: 0,
            selections: Vec::new(),
        }
    }

    pub fn open(path: &Path) -> Result<Document, OpenError> {
        let bytes = fs::read(path).map_err(|e| OpenError::Io(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|_| OpenError::NotUtf8)?;
        let line_ending = if text.contains("\r\n") { LineEnding::CRLF } else { LineEnding::LF };
        let normalized = text.replace("\r\n", "\n");
        Ok(Document {
            rope: Rope::from_str(&normalized),
            path: Some(path.to_path_buf()),
            dirty: false,
            line_ending,
            history: crate::undo::History::new(),
            scroll_row: 0,
            scroll_col: 0,
            cursor: 0,
            selections: Vec::new(),
        })
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or_else(|| "no path set".to_string())?;
        let text = self.rope.to_string();
        let text = match self.line_ending {
            LineEnding::LF => text,
            LineEnding::CRLF => text.replace('\n', "\r\n"),
        };
        fs::write(path, text).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    // ropey 1.6 uses char offsets/ranges natively — no byte conversion needed
    // (plan step 4 assumed older ropey semantics; compiler confirmed).
    pub fn edit(&mut self, at: usize, insert: &str, remove_len: usize) {
        if at > self.rope.len_chars() || remove_len > self.rope.len_chars() - at {
            panic!("edit range out of bounds");
        }
        let removed = if remove_len > 0 { self.slice(at, remove_len) } else { String::new() };
        if remove_len > 0 {
            self.rope.remove(at..at + remove_len);
        }
        if !insert.is_empty() {
            self.rope.insert(at, insert);
        }
        self.history.push(crate::undo::Change { at, inserted: insert.to_string(), removed });
        self.dirty = true;
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo(&mut self.rope).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo(&mut self.rope).is_some()
    }

    pub fn insert(&mut self, at: usize, text: &str) {
        self.edit(at, text, 0);
    }

    pub fn remove(&mut self, from: usize, len: usize) {
        self.edit(from, "", len);
    }

    pub fn slice(&self, from: usize, len: usize) -> String {
        self.rope.slice(from..from + len).to_string()
    }

    /// Non-empty selection range of the (single) active selection.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selections.last().and_then(|s| s.range())
    }
}

/// (line_start, line_end_excluding_terminator) char offsets for the line containing `off`.
pub fn line_bounds(rope: &Rope, off: usize) -> (usize, usize) {
    let line = rope.char_to_line(off.min(rope.len_chars()));
    let start = rope.line_to_char(line);
    let slice = rope.line(line);
    let mut len = slice.len_chars();
    if len > 0 && slice.chars().last() == Some('\n') {
        len -= 1;
    }
    (start, start + len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plat-t1-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn open_valid_file_yields_exact_text() {
        let p = tmp("valid.txt");
        fs::write(&p, "hello\nworld\n").unwrap();
        let doc = Document::open(&p).unwrap();
        assert_eq!(doc.rope.to_string(), "hello\nworld\n");
        assert_eq!(doc.path.as_deref(), Some(p.as_path()));
        assert!(!doc.dirty);
        fs::remove_file(&p).ok();
    }

    #[test]
    fn crlf_normalized_in_rope_and_restored_on_save() {
        let p = tmp("crlf.txt");
        fs::write(&p, "one\r\ntwo\r\n").unwrap();
        let mut doc = Document::open(&p).unwrap();
        assert_eq!(doc.line_ending, LineEnding::CRLF);
        assert_eq!(doc.rope.to_string(), "one\ntwo\n");
        doc.insert(3, "!");
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "one!\r\ntwo\r\n");
        assert!(!doc.dirty);
        fs::remove_file(&p).ok();
    }

    #[test]
    fn open_rejects_non_utf8() {
        let p = tmp("binary.bin");
        fs::write(&p, [0xFFu8, 0xFE, 0x00, 0x41]).unwrap();
        assert!(matches!(Document::open(&p), Err(OpenError::NotUtf8)));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn save_failure_keeps_buffer_dirty() {
        let p = tmp("readonly.txt");
        fs::write(&p, "abc").unwrap();
        let mut doc = Document::open(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        doc.insert(1, "x");
        assert!(doc.save().is_err());
        assert!(doc.dirty, "buffer must stay dirty after failed save");
        assert_eq!(doc.rope.to_string(), "axbc", "buffer content must survive failed save");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(&p).ok();
    }

    #[test]
    fn edit_records_history_undo_redo_at_document_level() {
        let mut doc = Document::from_str("ab");
        doc.edit(1, "x", 1); // replace 'b' with 'x'
        assert_eq!(doc.rope.to_string(), "ax");
        assert!(doc.undo());
        assert_eq!(doc.rope.to_string(), "ab");
        assert!(doc.redo());
        assert_eq!(doc.rope.to_string(), "ax");
        assert!(doc.undo());
        assert!(!doc.undo(), "no changes left to undo");
    }

    #[test]
    fn edit_marks_dirty_and_slice_returns_substring() {
        let mut doc = Document::from_str("héllo wörld");
        doc.insert(5, "X");
        assert_eq!(doc.slice(0, 6), "hélloX");
        assert!(doc.dirty);
        doc.remove(0, 1);
        assert_eq!(doc.slice(0, 1), "é");
    }

    // test-only convenience ctor
    impl Document {
        pub fn from_str(s: &str) -> Document {
            Document {
                rope: Rope::from_str(s),
                path: None,
                dirty: false,
                line_ending: LineEnding::LF,
                history: crate::undo::History::new(),
            scroll_row: 0,
            scroll_col: 0,
            cursor: 0,
            selections: Vec::new(),
            }
        }
    }
}
