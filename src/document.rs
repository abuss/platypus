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
    /// Char-offset (start, end) ranges for the last valid `search`.
    pub search_matches: Vec<(usize, usize)>,
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
        let old_path = self.path.clone();
        self.path = Some(path.to_path_buf());
        if let Err(e) = self.save() {
            self.path = old_path; // a failed save must not steal the old path
            return Err(e);
        }
        Ok(())
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

    /// Regex search over the whole document; matches are char-offset ranges.
    /// An invalid pattern is reported and the previous matches stay live, so
    /// live search while typing an incomplete regex never blanks the highlights.
    /// Zero-width matches (e.g. `\b`, `x*` at a non-x) are skipped: they would
    /// make stepping loop on the same position.
    /// ponytail: rebuilds a full String per search; revisit if 100k-line docs feel slow.
    pub fn set_search(&mut self, term: &str) -> Result<usize, String> {
        let re = match regex::Regex::new(term) {
            Ok(re) => re,
            Err(e) => return Err(e.to_string()),
        };
        self.search = Some(term.to_string());
        if term.is_empty() {
            self.search_matches.clear();
            return Ok(0);
        }
        let text = self.rope.slice(0..self.rope.len_chars()).to_string();
        let mut matches = Vec::new();
        // walk gaps between matches accumulating char counts: one pass total,
        // byte-offset -> char-offset without rescanning from 0 per match
        let mut last_byte = 0;
        let mut chars_seen = 0;
        for m in re.find_iter(&text) {
            if m.start() == m.end() {
                continue;
            }
            chars_seen += text[last_byte..m.start()].chars().count();
            let start = chars_seen;
            chars_seen += text[m.start()..m.end()].chars().count();
            matches.push((start, chars_seen));
            last_byte = m.end();
        }
        self.search_matches = matches;
        Ok(self.search_matches.len())
    }

    /// Move selection to the next match after/before the cursor; wraps at edges.
    pub fn search_next(&mut self, forward: bool) -> bool {
        if self.search.is_none() {
            return false;
        }
        let found = if forward {
            self.search_matches.iter().copied().find(|&(s, _)| s >= self.cursor)
                .or_else(|| self.search_matches.first().copied()) // wrap
        } else {
            self.search_matches.iter().copied().filter(|&(_, e)| e < self.cursor.max(1)).next_back()
                .or_else(|| self.search_matches.last().copied()) // wrap
        };
        match found {
            Some((s, e)) => {
                self.selections.clear();
                self.selections.push(Selection { anchor: s, head: e });
                self.cursor = e;
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
    fn search_wraps_around_edges() {
        let mut doc = Document::from_str("TARGET\nsecond line here\n");
        doc.set_search("TARGET").unwrap();
        doc.cursor = doc.rope.len_chars();
        assert!(doc.search_next(true), "forward from EOF wraps to the first match");
        assert_eq!(doc.selection(), Some((0, 6)));
        doc.cursor = 0;
        assert!(doc.search_next(false), "backward from start wraps to the last match");
        assert_eq!(doc.selection(), Some((0, 6)));
        doc.set_search("zzz").unwrap();
        assert!(!doc.search_next(true), "no matches at all: false");
    }

    #[test]
    fn regex_search_finds_pattern_with_variable_length_matches() {
        let mut doc = Document::from_str("a12b3 x");
        assert_eq!(doc.set_search(r"\d+").unwrap(), 2);
        assert_eq!(doc.search_matches, vec![(1, 3), (4, 5)]);
        doc.cursor = 0;
        doc.search_next(true);
        assert_eq!(doc.selection(), Some((1, 3)), "match 1 selected with its own length");
        doc.search_next(true);
        assert_eq!(doc.selection(), Some((4, 5)), "match 2 is one char long");
    }

    #[test]
    fn regex_search_offsets_are_chars_not_bytes() {
        let mut doc = Document::from_str("héllo wörld");
        assert_eq!(doc.set_search("l+").unwrap(), 2);
        assert_eq!(doc.search_matches, vec![(2, 4), (9, 10)], "char offsets across multibyte text");
    }

    #[test]
    fn invalid_regex_keeps_previous_matches_and_says_why() {
        let mut doc = Document::from_str("abc");
        doc.set_search("b").unwrap();
        let err = doc.set_search("(unclosed").unwrap_err();
        assert!(err.contains("unclosed") || err.contains("regex"), "error names the problem: {err}");
        assert_eq!(doc.search_matches, vec![(1, 2)], "previous matches survive an invalid pattern");
        assert_eq!(doc.search.as_deref(), Some("b"), "last valid pattern stays active");
    }

    #[test]
    fn empty_pattern_matches_nothing() {
        let mut doc = Document::from_str("abc");
        doc.set_search("b").unwrap();
        assert_eq!(doc.set_search("").unwrap(), 0);
        assert!(doc.search_matches.is_empty());
    }

    #[test]
    fn save_as_failure_restores_old_path() {
        let p = tmp("orig.txt");
        fs::write(&p, "keep").unwrap();
        let mut doc = Document::open(&p).unwrap();
        doc.insert(1, "x");
        let bad = tmp("no-such-dir/deep/new.txt");
        assert!(doc.save_as(&bad).is_err());
        assert_eq!(doc.path.as_deref(), Some(p.as_path()), "failed save_as must restore old path");
        assert!(doc.dirty);
        fs::remove_file(&p).ok();
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
