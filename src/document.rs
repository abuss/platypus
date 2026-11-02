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
    pub search: Option<String>,
    pub search_matches: Vec<usize>,
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
            search: None,
            search_matches: Vec::new(),
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
            search: None,
            search_matches: Vec::new(),
        })
    }

    pub fn save_as(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.path = Some(path.to_path_buf());
        self.save()
    }

    pub fn save(&mut self) -> Result<(), String> {        let path = self.path.as_ref().ok_or_else(|| "no path set".to_string())?;
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

    /// Literal substring search over the whole document; matches are char offsets.
    /// ponytail: rebuilds a full String per search; revisit if 100k-line docs feel slow.
    pub fn set_search(&mut self, term: &str) {
        let text = self.rope.slice(0..self.rope.len_chars()).to_string();
        let mut matches = Vec::new();
        let mut byte_i = 0;
        while let Some(pos) = text[byte_i..].find(term) {
            let at = byte_i + pos;
            matches.push(text[..at].chars().count());
            byte_i = at + term.len().max(1);
        }
        self.search = Some(term.to_string());
        self.search_matches = matches;
    }

    /// Move selection to the next match after/before the cursor. No wrap in Phase 0.
    pub fn search_next(&mut self, forward: bool) -> bool {
        let Some(term) = &self.search else { return false };
        let term_len = term.chars().count();
        let found = if forward {
            self.search_matches.iter().copied().find(|&p| p >= self.cursor)
        } else {
            self.search_matches.iter().copied().filter(|&p| p + term_len < self.cursor.max(1)).next_back()
        };
        match found {
            Some(p) => {
                self.selections.clear();
                self.selections.push(Selection { anchor: p, head: p + term_len });
                self.cursor = p + term_len;
                true
            }
            None => false,
        }
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

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Start of the next word after `off` (Helix-like: stops at each token run —
/// words, symbol runs, then lands on the next non-whitespace), or rope end.
pub fn word_next(rope: &Rope, off: usize) -> usize {
    let len = rope.len_chars();
    let mut i = off;
    if i >= len {
        return len;
    }
    let c = rope.char(i);
    if is_word(c) {
        while i < len && is_word(rope.char(i)) {
            i += 1;
        }
    } else if !c.is_whitespace() {
        while i < len && !is_word(rope.char(i)) && !rope.char(i).is_whitespace() {
            i += 1;
        }
    }
    while i < len && rope.char(i).is_whitespace() {
        i += 1;
    }
    i
}

/// Start of the previous word before `off`, or 0.
pub fn word_prev(rope: &Rope, off: usize) -> usize {
    if off == 0 {
        return 0;
    }
    let mut i = off;
    loop {
        i -= 1;
        if is_word(rope.char(i)) {
            break;
        }
        if i == 0 {
            return 0;
        }
    }
    while i > 0 && is_word(rope.char(i - 1)) {
        i -= 1;
    }
    i
}

/// Nearest occurrence of `ch` on the current line, searching right (or left)
/// of `off`. None when absent (motion must not move).
pub fn jump_char(rope: &Rope, off: usize, ch: char, backward: bool) -> Option<usize> {
    let (start, end) = line_bounds(rope, off);
    if backward {
        (start..off).rev().find(|&i| rope.char(i) == ch)
    } else {
        (off + 1..end).find(|&i| rope.char(i) == ch)
    }
}

/// Inside-range of the innermost `oc` pair enclosing `cursor` on its line:
/// brackets are depth-aware; quotes pair adjacent occurrences.
pub fn object_range(rope: &Rope, cursor: usize, oc: char) -> Option<(usize, usize)> {
    let (start, end) = line_bounds(rope, cursor);
    let close = match oc {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        c => c,
    };
    let line: Vec<char> = rope.slice(start..end).chars().collect();
    if close != oc {
        let mut stack: Vec<usize> = Vec::new();
        let mut best: Option<(usize, usize)> = None;
        for (i, c) in line.iter().enumerate() {
            if *c == oc {
                stack.push(start + i);
            } else if *c == close {
                if let Some(o) = stack.pop() {
                    if o < cursor && cursor < start + i {
                        best = Some((o + 1, start + i));
                    }
                }
            }
        }
        best
    } else {
        let positions: Vec<usize> =
            line.iter().enumerate().filter(|(_, c)| **c == oc).map(|(i, _)| start + i).collect();
        let mut best: Option<(usize, usize)> = None;
        for w in positions.windows(2) {
            if w[0] < cursor && cursor < w[1] && best.map_or(true, |(b, _)| w[0] > b) {
                best = Some((w[0] + 1, w[1]));
            }
        }
        best
    }
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
            search: None,
            search_matches: Vec::new(),
            }
        }
    }
}
