use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use tree_sitter::{InputEdit, Parser, Point, Tree};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Maps file extensions to language names (case-insensitive).
pub fn language_for(path: Option<&Path>) -> Option<&'static str> {
    let path = path?;
    let ext = path.extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "json" => "json",
        "c" | "h" => "c",
        "sh" | "bash" => "bash",
        _ => return None,
    })
}

/// Maps a scope to its style using longest-prefix matching.
/// Palette:
/// - keyword* → LightMagenta
/// - function.method / function → LightCyan
/// - type.builtin / type → Yellow
/// - string* / char → Green
/// - number / constant → LightBlue
/// - comment → DarkGray + ITALIC
/// - property / field → LightYellow
/// - error → Red
/// - everything else → None
pub fn style_for_scope(scope: &str) -> Option<Style> {
    let table = [
        ("keyword", Style::default().fg(Color::LightMagenta)),
        ("function.method", Style::default().fg(Color::LightCyan)),
        ("function", Style::default().fg(Color::LightCyan)),
        ("type.builtin", Style::default().fg(Color::Yellow)),
        ("type", Style::default().fg(Color::Yellow)),
        ("string", Style::default().fg(Color::Green)),
        ("char", Style::default().fg(Color::Green)),
        ("number", Style::default().fg(Color::LightBlue)),
        ("constant", Style::default().fg(Color::LightBlue)),
        ("comment", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ("property", Style::default().fg(Color::LightYellow)),
        ("field", Style::default().fg(Color::LightYellow)),
        ("error", Style::default().fg(Color::Red)),
    ];
    for (prefix, style) in &table {
        if scope.starts_with(prefix) {
            return Some(*style);
        }
    }
    None
}

/// Returns a tree_sitter::Language for the given language name.
pub fn grammar(lang: &'static str) -> Option<tree_sitter::Language> {
    Some(match lang {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "toml" => tree_sitter_toml_ng::LANGUAGE.into(),
        "markdown" => tree_sitter_md::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "bash" => tree_sitter_bash::LANGUAGE.into(),
        _ => return None,
    })
}

// ─── SyntaxState ─────────────────────────────────────────────────────────────

/// A pending edit: byte-offset half-open old range [start_byte, old_end_byte) replaced by `insert`.
pub struct PendingEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub insert: String,
}

/// A highlighted span for one viewport line (char columns).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSpan {
    pub line: usize,
    pub col: usize,
    pub end_col: usize,
    pub style: Style,
}

// Scope names registered with the highlighter (must stay in this order;
// highlight_index_to_style uses the index into this slice).
const HIGHLIGHT_NAMES: &[&str] = &[
    "keyword",
    "keyword.function",
    "keyword.operator",
    "keyword.return",
    "function",
    "function.method",
    "type",
    "type.builtin",
    "string",
    "string.special",
    "char",
    "number",
    "constant",
    "constant.builtin",
    "comment",
    "property",
    "field",
    "error",
];

fn highlight_index_to_style(index: usize) -> Option<Style> {
    style_for_scope(HIGHLIGHT_NAMES.get(index)?)
}

/// Build a HighlightConfiguration for the given language name.
fn build_config(lang: &'static str) -> Result<HighlightConfiguration, String> {
    let language = grammar(lang).ok_or_else(|| format!("unknown language: {lang}"))?;
    let (highlights_query, injection_query) = match lang {
        "rust" => (tree_sitter_rust::HIGHLIGHTS_QUERY, tree_sitter_rust::INJECTIONS_QUERY),
        "toml" => (tree_sitter_toml_ng::HIGHLIGHTS_QUERY, ""),
        "markdown" => (tree_sitter_md::HIGHLIGHT_QUERY_BLOCK, tree_sitter_md::INJECTION_QUERY_BLOCK),
        "json" => (tree_sitter_json::HIGHLIGHTS_QUERY, ""),
        "c" => (tree_sitter_c::HIGHLIGHT_QUERY, ""),
        "bash" => (tree_sitter_bash::HIGHLIGHT_QUERY, ""),
        _ => return Err(format!("unknown language: {lang}")),
    };
    // tree-sitter-highlight 0.25 HighlightConfiguration::new takes (language, name, highlights, injections, locals)
    HighlightConfiguration::new(language, lang, highlights_query, injection_query, "")
        .map_err(|e| format!("query error for {lang}: {e}"))
}

/// Compute the tree-sitter Point (row, byte-col) for a byte offset in a string.
fn byte_offset_to_point(text: &str, byte: usize) -> Point {
    let before = &text[..byte.min(text.len())];
    let row = before.bytes().filter(|&b| b == b'\n').count();
    let col = before.rfind('\n').map(|p| byte - p - 1).unwrap_or(byte);
    Point::new(row, col)
}

/// Holds incremental parse state and produces FrameSpans for a viewport.
pub struct SyntaxState {
    shadow: String,
    config: HighlightConfiguration,
    parser: Parser,
    tree: Option<Tree>,
    last_view: Vec<(usize, usize, usize)>,
    spans: Vec<FrameSpan>,
}

impl SyntaxState {
    pub fn build(lang: &'static str, text: &str) -> Result<Self, String> {
        let language = grammar(lang).ok_or_else(|| format!("unknown language: {lang}"))?;
        let mut config = build_config(lang)?;
        config.configure(HIGHLIGHT_NAMES);

        let mut parser = Parser::new();
        parser.set_language(&language).map_err(|e| format!("set_language: {e}"))?;
        let tree = parser.parse(text, None);

        Ok(Self {
            shadow: text.to_string(),
            config,
            parser,
            tree,
            last_view: Vec::new(),
            spans: Vec::new(),
        })
    }

    /// Apply edits and re-highlight the given viewport lines.
    /// `view`: (line_index, byte_start, byte_end) — inclusive byte range of each visible line.
    /// Skips re-query when edits is empty and view equals last view (frame cache).
    pub fn refresh(&mut self, edits: &[PendingEdit], view: &[(usize, usize, usize)]) {
        if edits.is_empty() && view == self.last_view.as_slice() {
            return;
        }

        for edit in edits {
            let start = edit.start_byte.min(self.shadow.len());
            let old_end = edit.old_end_byte.min(self.shadow.len());

            let start_pos = byte_offset_to_point(&self.shadow, start);
            let old_end_pos = byte_offset_to_point(&self.shadow, old_end);

            self.shadow.replace_range(start..old_end, &edit.insert);

            let new_end_byte = start + edit.insert.len();
            let new_end_pos = byte_offset_to_point(&self.shadow, new_end_byte);

            let input_edit = InputEdit {
                start_byte: start,
                old_end_byte: old_end,
                new_end_byte,
                start_position: start_pos,
                old_end_position: old_end_pos,
                new_end_position: new_end_pos,
            };

            if let Some(tree) = &mut self.tree {
                tree.edit(&input_edit);
            }
        }

        // Incremental reparse (tree already has edits applied)
        self.tree = self.parser.parse(self.shadow.as_bytes(), self.tree.as_ref());

        self.last_view = view.to_vec();
        self.spans = self.compute_spans(view);
    }

    fn compute_spans(&self, view: &[(usize, usize, usize)]) -> Vec<FrameSpan> {
        let mut highlighter = Highlighter::new();
        let source = self.shadow.as_bytes();
        let events = match highlighter.highlight(&self.config, source, None, |_| None) {
            Ok(it) => it,
            Err(_) => return Vec::new(),
        };

        let mut spans = Vec::new();
        let mut current_style: Option<Style> = None;

        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => {
                    current_style = highlight_index_to_style(h.0);
                }
                Ok(HighlightEvent::HighlightEnd) => {
                    current_style = None;
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    let style = match current_style {
                        Some(s) => s,
                        None => continue,
                    };
                    for &(line_idx, line_byte_start, line_byte_end) in view {
                        if end <= line_byte_start || start > line_byte_end {
                            continue;
                        }
                        let seg_start = start.max(line_byte_start);
                        // line_byte_end is inclusive; end is exclusive
                        let seg_end = end.min(line_byte_end + 1);
                        if seg_start >= seg_end {
                            continue;
                        }
                        let line_len = (line_byte_end + 1).min(self.shadow.len()) - line_byte_start;
                        let line_text = &self.shadow[line_byte_start..line_byte_start + line_len];
                        let col = byte_to_char_col(line_text, seg_start - line_byte_start);
                        let end_col = byte_to_char_col(line_text, seg_end - line_byte_start);
                        if col < end_col {
                            spans.push(FrameSpan { line: line_idx, col, end_col, style });
                        }
                    }
                }
                Err(_) => break,
            }
        }

        spans
    }

    pub fn spans(&self) -> &[FrameSpan] {
        &self.spans
    }

    pub fn text(&self) -> &str {
        &self.shadow
    }
}

/// Convert a byte offset within a line's text to a char column.
fn byte_to_char_col(line_text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(line_text.len());
    line_text[..clamped].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_map_is_case_insensitive() {
        assert_eq!(language_for(Some(Path::new("a.RS"))), Some("rust"));
        assert_eq!(language_for(Some(Path::new("x/README.MD"))), Some("markdown"));
        assert_eq!(language_for(Some(Path::new("k.h"))), Some("c"));
        assert_eq!(language_for(Some(Path::new("notes.txt"))), None);
        assert_eq!(language_for(None), None);
    }

    #[test]
    fn palette_scopes_resolve() {
        assert_eq!(
            style_for_scope("keyword.function"),
            Some(Style::default().fg(Color::LightMagenta))
        );
        assert_eq!(
            style_for_scope("comment"),
            Some(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))
        );
        assert_eq!(style_for_scope("variable"), None);
        assert_eq!(style_for_scope("nonexistent.scope"), None);
        assert_eq!(style_for_scope("punctuation.bracket"), None);
        assert_eq!(style_for_scope("punctuation.delimiter"), None);
    }

    #[test]
    fn all_six_grammars_construct() {
        for name in ["rust", "toml", "markdown", "json", "c", "bash"] {
            assert!(grammar(name).is_some(), "{name} missing");
        }
    }

    /// Build (line, byte_start, byte_end) view tuples for lines [from..=to] of text.
    fn view_lines(from: usize, to: usize, text: &str) -> Vec<(usize, usize, usize)> {
        let mut result = Vec::new();
        let mut byte = 0usize;
        for (i, line) in text.split_inclusive('\n').enumerate() {
            if i > to {
                break;
            }
            let line_start = byte;
            // byte_end is the last byte of the line content (excluding the \n)
            let line_end = (byte + line.len()).saturating_sub(1);
            byte += line.len();
            if i >= from {
                result.push((i, line_start, line_end));
            }
        }
        result
    }

    #[test]
    fn rust_keyword_spans_appear_on_their_line() {
        let mut s = SyntaxState::build("rust", "fn main() {}\n").unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        let fn_span = s.spans().iter().find(|f| f.line == 0 && f.col == 0).unwrap();
        assert_eq!(fn_span.style.fg, Some(Color::LightMagenta));
    }

    #[test]
    fn spans_use_char_cols_on_multibyte_lines() {
        // Line 0: "// é\n" — é is 2 bytes in UTF-8
        // Line 1: "fn main() {}\n"
        let src = "// é\nfn main() {}\n";
        let mut s = SyntaxState::build("rust", src).unwrap();
        let view = view_lines(0, 1, s.text());
        s.refresh(&[], &view);
        // Find "fn" keyword on line 1
        let l1 = s
            .spans()
            .iter()
            .find(|f| f.line == 1 && f.style.fg == Some(Color::LightMagenta))
            .expect("keyword span on line 1");
        assert_eq!(l1.col, 0, "fn at col 0");
        assert_eq!(l1.end_col, 2, "fn is 2 chars");
    }

    #[test]
    fn incremental_matches_fresh_rebuild() {
        let src = "fn main() {}\n";
        let mut inc = SyntaxState::build("rust", src).unwrap();
        let view0 = view_lines(0, 0, inc.text());
        inc.refresh(&[], &view0);

        // Insert "pub " at byte 0
        let e1 = PendingEdit {
            start_byte: 0,
            old_end_byte: 0,
            insert: "pub ".to_string(),
        };
        let new_text = "pub fn main() {}\n";
        inc.refresh(&[e1], &view_lines(0, 0, new_text));
        assert_eq!(inc.text(), new_text);

        // Compare incremental spans vs fresh build on same view
        let mut fresh = SyntaxState::build("rust", inc.text()).unwrap();
        let view = view_lines(0, 0, fresh.text());
        fresh.refresh(&[], &view);
        // Refresh inc with the exact same view so both see the same viewport
        inc.refresh(&[], &view);

        let to_key = |spans: &[FrameSpan]| -> std::collections::BTreeSet<(usize, usize, usize, String)> {
            spans
                .iter()
                .map(|s| (s.line, s.col, s.end_col, format!("{:?}", s.style.fg)))
                .collect()
        };
        assert_eq!(to_key(inc.spans()), to_key(fresh.spans()));
    }

    #[test]
    fn unknown_language_is_an_error_not_a_panic() {
        assert!(SyntaxState::build("klingon", "x").is_err());
    }

    #[test]
    fn frame_cache_skips_requery_same_view() {
        let mut s = SyntaxState::build("rust", "fn main() {}\n").unwrap();
        let view_a = view_lines(0, 0, s.text());
        s.refresh(&[], &view_a);
        let spans_a: Vec<_> = s.spans().to_vec();
        // Same view again → should be a no-op (cache hit)
        s.refresh(&[], &view_a);
        assert_eq!(s.spans(), spans_a.as_slice());
    }
}
