use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use tree_sitter::{InputEdit, Parser, Point, Tree};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Feature gate: Task 5 will replace this with a config field.
pub const SYNTAX_DEFAULT: bool = true;

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

/// A pending edit: byte-offset half-open old range [start_byte, end_byte) replaced by `insert`.
pub struct PendingEdit {
    pub start_byte: usize,
    pub end_byte: usize,
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
    // tree-sitter-highlight 0.25: new(language, name, highlights, injections, locals)
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
    /// Incremented each time a highlight query actually runs (skipped on frame-cache hit).
    pub query_count: usize,
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
            query_count: 0,
        })
    }

    /// Apply edits and re-highlight the given viewport lines.
    /// `view`: (line_index, byte_start, byte_end) — byte range of each visible line,
    ///   where byte_end is the last byte BEFORE the newline (exclusive of `\n`).
    /// Skips re-query when edits is empty and view equals last view (frame cache).
    pub fn refresh(&mut self, edits: &[PendingEdit], view: &[(usize, usize, usize)]) {
        if edits.is_empty() && view == self.last_view.as_slice() {
            return;
        }

        for edit in edits {
            let start = edit.start_byte.min(self.shadow.len());
            let old_end = edit.end_byte.min(self.shadow.len());

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
        self.query_count += 1;
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
                        // line_byte_end is exclusive: [line_byte_start, line_byte_end)
                        if end <= line_byte_start || start >= line_byte_end {
                            continue;
                        }
                        let seg_start = start.max(line_byte_start);
                        let seg_end = end.min(line_byte_end).min(self.shadow.len());
                        if seg_start >= seg_end {
                            continue;
                        }
                        // line_text covers only the printable content (no \n).
                        let line_content_len = line_byte_end - line_byte_start;
                        let safe_end = (line_byte_start + line_content_len).min(self.shadow.len());
                        if line_byte_start > safe_end {
                            continue;
                        }
                        let line_text = &self.shadow[line_byte_start..safe_end];
                        // Char count of the line (without \n) for clamping end_col.
                        let line_char_len = line_text.chars().count();
                        let col = byte_to_char_col(line_text, seg_start - line_byte_start);
                        let end_col = byte_to_char_col(line_text, seg_end - line_byte_start)
                            .min(line_char_len); // never cover the \n cell
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

    /// Build (line, byte_start, byte_end_exclusive) view tuples for lines [from..=to] of text.
    /// byte_end is exclusive: [byte_start, byte_end) covers only the line content, no \n.
    fn view_lines(from: usize, to: usize, text: &str) -> Vec<(usize, usize, usize)> {
        let mut result = Vec::new();
        let mut byte = 0usize;
        for (i, line) in text.split_inclusive('\n').enumerate() {
            if i > to {
                break;
            }
            let line_start = byte;
            // Strip the trailing \n from the byte length before computing end.
            let content_len = if line.ends_with('\n') { line.len() - 1 } else { line.len() };
            // byte_end is exclusive: line_start + content_len
            let line_end = line_start + content_len;
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
        // No span may cover the \n cell (char index 13 for "fn main() {}")
        let line_char_len = "fn main() {}".chars().count(); // 12
        assert!(
            s.spans().iter().all(|sp| sp.line != 0 || sp.end_col <= line_char_len),
            "a span covers the \\n cell"
        );
    }

    #[test]
    fn spans_use_char_cols_on_multibyte_lines() {
        // Line 0: "// é\n" — é is 2 bytes in UTF-8 (U+00E9)
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
        // Source has a multibyte char above the edited line to stress byte↔char accounting.
        // Line 0 (comment): "// café\n"   — 'é' = 2 bytes, 'f' before it is ASCII
        // Line 1 (code):    "fn main() {}\n"
        let src = "// café\nfn main() {}\n";

        let mut inc = SyntaxState::build("rust", src).unwrap();
        inc.refresh(&[], &view_lines(0, 1, inc.text()));

        // Edit (a): insert "pub " at byte 0 → "pub // café\nfn main() {}\n"
        // Edit (b): delete the 3 bytes of "pub" (bytes 0..3) → "  // café\nfn main() {}\n"
        //           Wait — we apply in sequence on the current shadow, so after (a) shadow
        //           is "pub // café\nfn main() {}\n". Edit (b) deletes bytes [0,3) → " // café\n…"
        //           but we want a clean "fn" test; let's instead do:
        //
        // Edit (a): insert "pub " at byte 0              → "pub // café\nfn main() {}\n"
        // Edit (b): delete bytes [0,4) (removes "pub ")  → "// café\nfn main() {}\n"  (back to src)
        // Edit (c): insert "x" right after 'é' at the multibyte boundary.
        //           In "// café\n", 'é' is at bytes 5..7 (UTF-8: 0xC3 0xA9).
        //           After edit (b) shadow == src, so 'é' ends at byte 7.
        //           Insert "x" at byte 7 (right after 'é') → "// caféx\nfn main() {}\n"
        //
        // After all three edits, final text = "// caféx\nfn main() {}\n".

        let e_a = PendingEdit { start_byte: 0, end_byte: 0, insert: "pub ".to_string() };
        // After (a): "pub // café\nfn main() {}\n"
        // Edit (b) deletes [0,4) in the new shadow
        let e_b = PendingEdit { start_byte: 0, end_byte: 4, insert: String::new() };
        // After (b): "// café\nfn main() {}\n"  (same as src)
        // "// café": bytes 0='/', 1='/', 2=' ', 3='c', 4='a', 5='f', 6..8='é' (U+00E9, 2 bytes)
        // Insert "x" at byte 8 (right after 'é') → "// caféx\nfn main() {}\n"
        let e_c = PendingEdit { start_byte: 8, end_byte: 8, insert: "x".to_string() };
        // After (c): "// caféx\nfn main() {}\n"
        let final_text = "// caféx\nfn main() {}\n";

        let view = view_lines(0, 1, final_text);
        inc.refresh(&[e_a, e_b, e_c], &view);
        assert_eq!(inc.text(), final_text, "shadow after three edits");

        // Fresh build on the final text
        let mut fresh = SyntaxState::build("rust", final_text).unwrap();
        fresh.refresh(&[], &view);

        let to_key = |spans: &[FrameSpan]| -> std::collections::BTreeSet<(usize, usize, usize, String)> {
            spans
                .iter()
                .map(|s| (s.line, s.col, s.end_col, format!("{:?}", s.style.fg)))
                .collect()
        };
        assert_eq!(
            to_key(inc.spans()),
            to_key(fresh.spans()),
            "incremental spans must match fresh rebuild"
        );
    }

    #[test]
    fn unknown_language_is_an_error_not_a_panic() {
        assert!(SyntaxState::build("klingon", "x").is_err());
    }

    #[test]
    fn frame_cache_skips_requery_same_view() {
        let src = "fn main() {}\n";
        let mut s = SyntaxState::build("rust", src).unwrap();
        let view_a = view_lines(0, 0, src);
        let view_b = view_lines(0, 0, src); // same content, different Vec

        // First refresh — query fires
        s.refresh(&[], &view_a);
        assert_eq!(s.query_count, 1, "initial refresh must run query");

        // Same view again — cache hit, query must NOT fire
        s.refresh(&[], &view_b);
        assert_eq!(s.query_count, 1, "same-view refresh must be a no-op");

        // Edit → cache invalidated, query fires
        let edit = PendingEdit { start_byte: 0, end_byte: 0, insert: "// hi\n".to_string() };
        let new_text = format!("// hi\n{src}");
        let view_c = view_lines(0, 1, &new_text);
        s.refresh(&[edit], &view_c);
        assert_eq!(s.query_count, 2, "edit must trigger re-query");

        // Same view_c again — cache hit
        let view_c2 = view_lines(0, 1, &new_text);
        s.refresh(&[], &view_c2);
        assert_eq!(s.query_count, 2, "same view after edit must be cached");
    }

    #[test]
    fn empty_lines_no_panic_and_sane_spans() {
        // "fn x(){}\n\nfn y(){}\n" — line 1 is empty, line 3 is the phantom final line.
        let src = "fn x(){}\n\nfn y(){}\n";
        let mut s = SyntaxState::build("rust", src).unwrap();
        // view_lines(0,2) covers lines 0,1,2; splits on \n so line 3 (phantom after final \n) is excluded.
        let view = view_lines(0, 2, src);
        // Must not panic
        s.refresh(&[], &view);
        // Empty line 1 must produce no spans
        let line1_spans: Vec<_> = s.spans().iter().filter(|sp| sp.line == 1).collect();
        assert!(line1_spans.is_empty(), "empty line must have no spans: {:?}", line1_spans);
        // Lines 0 and 2 have keywords
        assert!(s.spans().iter().any(|sp| sp.line == 0), "line 0 must have keyword spans");
    }
}
