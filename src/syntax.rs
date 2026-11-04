use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use tree_sitter::{InputEdit, Parser, Point, QueryCursor, Tree};
use tree_sitter_highlight::HighlightConfiguration;

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
        let tree = match &self.tree {
            Some(t) => t,
            None => return Vec::new(),
        };
        if view.is_empty() {
            return Vec::new();
        }

        // Run the highlight query only over the viewport byte range.
        let view_first_byte = view[0].1;
        let view_last_byte = view[view.len() - 1].2;

        // Build a capture-name → Style map from the query's capture names.
        // This is O(captures × styles) but tiny.
        let capture_styles: Vec<Option<Style>> = self
            .config
            .query
            .capture_names()
            .iter()
            .map(|name| style_for_scope(name))
            .collect();

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(view_first_byte..view_last_byte.max(view_first_byte + 1));
        let source = self.shadow.as_bytes();
        let root = tree.root_node();
        let mut captures = cursor.captures(&self.config.query, root, source);

        let mut spans = Vec::new();
        while let Some((mat, capture_idx)) = {
            use tree_sitter::StreamingIterator;
            captures.next()
        } {
            let cap = &mat.captures[*capture_idx];
            let style = match capture_styles.get(cap.index as usize).and_then(|s| *s) {
                Some(s) => s,
                None => continue,
            };
            let start = cap.node.start_byte();
            let end = cap.node.end_byte();

            // Binary-search to the first view line that could overlap [start, end).
            let first = view.partition_point(|&(_, _, be)| be <= start);
            for &(line_idx, line_byte_start, line_byte_end) in &view[first..] {
                if line_byte_start >= end {
                    break;
                }
                let seg_start = start.max(line_byte_start);
                let seg_end = end.min(line_byte_end).min(self.shadow.len());
                if seg_start >= seg_end {
                    continue;
                }
                let safe_end = line_byte_end.min(self.shadow.len());
                if line_byte_start > safe_end {
                    continue;
                }
                let line_text = &self.shadow[line_byte_start..safe_end];
                let line_char_len = line_text.chars().count();
                let col = byte_to_char_col(line_text, seg_start - line_byte_start);
                let end_col = byte_to_char_col(line_text, seg_end - line_byte_start)
                    .min(line_char_len);
                if col < end_col {
                    spans.push(FrameSpan { line: line_idx, col, end_col, style });
                }
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

    #[test]
    fn rust_keyword_spans_appear_on_their_line() {
        let mut s = SyntaxState::build("rust", "fn main() {}\n").unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        let fn_span = s.spans().iter().find(|f| f.line == 0 && f.col == 0).unwrap();
        assert_eq!(fn_span.style.fg, Some(Color::LightMagenta));
        let line_char_len = "fn main() {}".chars().count(); // 12
        assert!(
            s.spans().iter().all(|sp| sp.line != 0 || sp.end_col <= line_char_len),
            "a span covers the \n cell"
        );
    }

    #[test]
    fn spans_use_char_cols_on_multibyte_lines() {
        let src = "// é\nfn main() {}\n";
        let mut s = SyntaxState::build("rust", src).unwrap();
        let view = view_lines(0, 1, s.text());
        s.refresh(&[], &view);
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
        let src = "// café\nfn main() {}\n";

        let mut inc = SyntaxState::build("rust", src).unwrap();
        inc.refresh(&[], &view_lines(0, 1, inc.text()));

        let e_a = PendingEdit { start_byte: 0, end_byte: 0, insert: "pub ".to_string() };
        let e_b = PendingEdit { start_byte: 0, end_byte: 4, insert: String::new() };
        // "// café": 'é' ends at byte 8 (U+00E9, 2 bytes at bytes 6..8)
        let e_c = PendingEdit { start_byte: 8, end_byte: 8, insert: "x".to_string() };
        let final_text = "// caféx\nfn main() {}\n";

        let view = view_lines(0, 1, final_text);
        inc.refresh(&[e_a, e_b, e_c], &view);
        assert_eq!(inc.text(), final_text, "shadow after three edits");

        let mut fresh = SyntaxState::build("rust", final_text).unwrap();
        fresh.refresh(&[], &view);

        let to_key = |spans: &[FrameSpan]| -> std::collections::BTreeSet<(usize, usize, usize, String)> {
            spans.iter().map(|s| (s.line, s.col, s.end_col, format!("{:?}", s.style.fg))).collect()
        };
        assert_eq!(to_key(inc.spans()), to_key(fresh.spans()), "incremental spans must match fresh rebuild");
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
        let view_b = view_lines(0, 0, src);

        s.refresh(&[], &view_a);
        assert_eq!(s.query_count, 1, "initial refresh must run query");

        s.refresh(&[], &view_b);
        assert_eq!(s.query_count, 1, "same-view refresh must be a no-op");

        let edit = PendingEdit { start_byte: 0, end_byte: 0, insert: "// hi\n".to_string() };
        let new_text = format!("// hi\n{src}");
        let view_c = view_lines(0, 1, &new_text);
        s.refresh(&[edit], &view_c);
        assert_eq!(s.query_count, 2, "edit must trigger re-query");

        let view_c2 = view_lines(0, 1, &new_text);
        s.refresh(&[], &view_c2);
        assert_eq!(s.query_count, 2, "same view after edit must be cached");
    }

    /// Build (line, byte_start, byte_end_exclusive) view tuples for lines [from..=to] of text.
    /// byte_end is exclusive: [byte_start, byte_end) covers only the line content, no \n.
    fn view_lines(from: usize, to: usize, text: &str) -> Vec<(usize, usize, usize)> {
        let mut result = Vec::new();
        let mut byte = 0usize;
        for (i, line) in text.split_inclusive('\n').enumerate() {
            if i > to { break; }
            let line_start = byte;
            let content_len = if line.ends_with('\n') { line.len() - 1 } else { line.len() };
            let line_end = line_start + content_len;
            byte += line.len();
            if i >= from { result.push((i, line_start, line_end)); }
        }
        result
    }

    // ─── Perf gates (run with: cargo test --release -- --ignored syntax) ─────────
    //
    // R2 policy (binding):
    //   build          ≤ 800 ms  (one-off initial parse; async-parse is future work)
    //   mid-file edit  ≤  10 ms  (keystroke path — HARD GATE)
    //   byte-0 edit    no gate   (worst case; printed for reference)
    //   warm viewport  ≤   2 ms  (frame-cache hit)
    //
    // Root-cause note (diagnosed 2026-11-03):
    //   parser.parse (incremental) on 100k Rust lines ≈ 50 ms regardless of edit
    //   position or whether the parse tree changes (changed_ranges=0 for no-op edits).
    //   QueryCursor (viewport-only, set_byte_range) ≈ 256 µs — not the bottleneck.
    //   The 10 ms mid-file gate CANNOT be met without async parsing or file-size cutoff.
    //   Gate asserted so controller sees the actual failure; policy decision pending.

    /// R2 gate: SyntaxState::build on 100k-line Rust source ≤ 800 ms.
    #[test]
    #[ignore]
    fn bench_syntax_build_100k_lines() {
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let src = line.repeat(100_000);
        let t = std::time::Instant::now();
        let _ = SyntaxState::build("rust", &src).unwrap();
        let elapsed = t.elapsed();
        println!("bench_syntax_build_100k_lines: {elapsed:?}");
        assert!(elapsed.as_millis() < 800, "build too slow: {elapsed:?}");
    }

    /// R2 gate: mid-file realistic edit (Enter key at line 50000) ≤ 10 ms.
    /// Diagnosed: parser.parse itself ≈ 50 ms on 100k Rust lines (tree-sitter inherent).
    /// Gate FAILS. Root cause: library tree-walk cost, not a code bug. Policy pending.
    #[test]
    #[ignore]
    fn bench_syntax_mid_file_edit_refresh_100k_lines() {
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let src = line.repeat(100_000);
        let mid_byte = src.len() / 2;
        let mid_byte = src[mid_byte..].find('\n').map(|i| mid_byte + i + 1).unwrap_or(mid_byte);
        let mid_line = mid_byte / line.len();

        let mut s = SyntaxState::build("rust", &src).unwrap();
        s.refresh(&[], &view_lines(mid_line, mid_line + 59, &src));

        let edit = PendingEdit { start_byte: mid_byte, end_byte: mid_byte, insert: "\n".to_string() };
        let new_src = format!("{}\n{}", &src[..mid_byte], &src[mid_byte..]);
        let new_view = view_lines(mid_line, mid_line + 59, &new_src);
        let t = std::time::Instant::now();
        s.refresh(&[edit], &new_view);
        let elapsed = t.elapsed();
        println!("bench_syntax_mid_file_edit_refresh_100k_lines: {elapsed:?}");
        assert!(elapsed.as_millis() < 10, "mid-file edit refresh too slow: {elapsed:?}");
    }

    /// Byte-0 edit — no gate (documented worst case; parser.parse cost ≈ 50 ms irreducible).
    #[test]
    #[ignore]
    fn bench_syntax_byte0_edit_refresh_100k_lines() {
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let src = line.repeat(100_000);
        let mut s = SyntaxState::build("rust", &src).unwrap();
        s.refresh(&[], &view_lines(0, 59, &src));
        let edit = PendingEdit { start_byte: 0, end_byte: 0, insert: " ".to_string() };
        let new_src = format!(" {src}");
        let t = std::time::Instant::now();
        s.refresh(&[edit], &view_lines(0, 59, &new_src));
        let elapsed = t.elapsed();
        println!("bench_syntax_byte0_edit_refresh_100k_lines (no gate): {elapsed:?}");
    }

    /// R2 gate: warm same-view refresh (frame cache hit) ≤ 2 ms.
    #[test]
    #[ignore]
    fn bench_syntax_warm_viewport_100k_lines() {
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let src = line.repeat(100_000);
        let mut s = SyntaxState::build("rust", &src).unwrap();
        let view = view_lines(0, 59, &src);
        s.refresh(&[], &view);
        let view2 = view_lines(0, 59, &src);
        let t = std::time::Instant::now();
        s.refresh(&[], &view2);
        let elapsed = t.elapsed();
        println!("bench_syntax_warm_viewport_100k_lines: {elapsed:?}");
        assert!(elapsed.as_millis() < 2, "warm viewport too slow: {elapsed:?}");
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
