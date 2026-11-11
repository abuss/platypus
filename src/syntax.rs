use ratatui::style::Style;
use std::path::Path;
use tree_sitter::{InputEdit, Parser, Point, QueryCursor, Range, Tree};
use tree_sitter_highlight::HighlightConfiguration;

/// Lines of context added on each side of the viewport for the parse window.
const PARSE_MARGIN_LINES: usize = 2000;

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
        "py" | "pyi" => "python",
        "go" => "go",
        "lua" => "lua",
        _ => return None,
    })
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
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "lua" => tree_sitter_lua::LANGUAGE.into(),
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

// Scope names registered with the highlighter (matched by name in queries).
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
    // scopes produced by the python/go/lua grammars' highlight queries
    // (all capture names must be registered or HighlightConfiguration::new fails)
    "attribute",
    "boolean",
    "conditional",
    "constructor",
    "embedded",
    "escape",
    "function.builtin",
    "function.call",
    "label",
    "method",
    "method.call",
    "operator",
    "parameter",
    "preproc",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "repeat",
    "string.escape",
    "variable",
    "variable.builtin",
];

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
        "python" => (tree_sitter_python::HIGHLIGHTS_QUERY, ""),
        "go" => (tree_sitter_go::HIGHLIGHTS_QUERY, ""),
        "lua" => (tree_sitter_lua::HIGHLIGHTS_QUERY, tree_sitter_lua::INJECTIONS_QUERY),
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

/// Human label for structural nodes (fn/class/impl/struct/enum/trait),
/// "kind name" via the name/type field; None for everything else (#56).
fn symbol_label(node: tree_sitter::Node, bytes: &[u8]) -> Option<String> {
    let (kind, name_field): (&str, Option<&str>) = match node.kind() {
        "function" | "function_declaration" | "function_signature" | "function_item"
        | "function_definition" | "method" | "method_definition" | "method_declaration" => ("fn", Some("name")),
        "class" | "class_definition" | "class_specifier" => ("class", Some("name")),
        "impl_item" => ("impl", Some("type")),
        "struct" | "struct_item" | "struct_specifier" => ("struct", Some("name")),
        "enum" | "enum_item" | "enum_specifier" => ("enum", Some("name")),
        "trait_item" | "interface" => ("trait", Some("name")),
        _ => return None,
    };
    let name = name_field
        .and_then(|f| node.child_by_field_name(f))
        .and_then(|n| n.utf8_text(bytes).ok())
        .unwrap_or("");
    Some(if name.is_empty() { kind.to_string() } else { format!("{kind} {name}") })
}

/// Holds incremental parse state and produces FrameSpans for a viewport.
pub struct SyntaxState {
    shadow: String,
    config: HighlightConfiguration,
    parser: Parser,
    tree: Option<Tree>,
    last_view: Vec<(usize, usize, usize)>,
    /// Last parse-area byte range [start, end); None = never parsed yet.
    last_area: Option<(usize, usize)>,
    spans: Vec<FrameSpan>,
    /// Incremented each time a highlight query actually runs (skipped on frame-cache hit).
    pub query_count: usize,
    /// Resolved syntax palette (cloned from App at build time).
    palette: crate::theme::SyntaxPalette,
}

impl SyntaxState {
    /// The enclosing symbol chain at a point, outer->inner, max 3 innermost
    /// labels (#56 breadcrumb): "impl App" -> "fn handle_command".
    pub fn symbol_chain(&self, pt: tree_sitter::Point) -> Vec<String> {
        let Some(tree) = &self.tree else { return Vec::new() };
        let bytes = self.shadow.as_bytes();
        let mut out = Vec::new();
        let mut cur = tree.root_node().descendant_for_point_range(pt, pt);
        while let Some(n) = cur {
            if let Some(label) = symbol_label(n, bytes) {
                out.push(label);
                if out.len() >= 3 {
                    break;
                }
            }
            cur = n.parent();
        }
        out.reverse();
        out
    }
    pub fn build(lang: &'static str, text: &str, palette: &crate::theme::Palette) -> Result<Self, String> {
        let language = grammar(lang).ok_or_else(|| format!("unknown language: {lang}"))?;
        let mut config = build_config(lang)?;
        config.configure(HIGHLIGHT_NAMES);

        let mut parser = Parser::new();
        parser.set_language(&language).map_err(|e| format!("set_language: {e}"))?;
        // Defer the initial parse to the first refresh(), where it will be
        // windowed to the viewport. No eager full-file parse.

        Ok(Self {
            shadow: text.to_string(),
            config,
            parser,
            tree: None,
            last_view: Vec::new(),
            last_area: None,
            spans: Vec::new(),
            query_count: 0,
            palette: palette.syntax.clone(),
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

        // Compute the parse window: view ± PARSE_MARGIN_LINES, clamped to file bounds.
        let area = parse_area(&self.shadow, view, PARSE_MARGIN_LINES);

        // Only call set_included_ranges when the area changed — docs warn changing
        // ranges can force extra work on the next parse.
        if self.last_area != Some(area) {
            let (ab, ae) = area;
            // Compute Points efficiently: scan only the margin region, not from byte 0.
            // view[0].1 is a known boundary; count newlines from ab to it (bounded by margin).
            let view_start_byte = view[0].1;
            let view_start_row  = view[0].0; // known from the view tuple
            let ab_row = view_start_row.saturating_sub(
                self.shadow.as_bytes()[ab..view_start_byte]
                    .iter().filter(|&&b| b == b'\n').count()
            );
            // area_start is always a line start, so col is 0
            let ab_pt = Point::new(ab_row, 0);

            let view_end_byte = view[view.len() - 1].2;
            let view_end_row  = view[view.len() - 1].0;
            let ae_row = view_end_row +
                self.shadow.as_bytes()[view_end_byte..ae.min(self.shadow.len())]
                    .iter().filter(|&&b| b == b'\n').count();
            let ae_col = 0usize;
            let ae_pt = Point::new(ae_row, ae_col);

            let range = Range {
                start_byte:  ab,
                end_byte:    ae,
                start_point: ab_pt,
                end_point:   ae_pt,
            };
            // error unreachable: single range, start<end by construction; if error, tree-sitter keeps previous ranges
            let _ = self.parser.set_included_ranges(&[range]);
            self.last_area = Some(area);
        }

        // Incremental windowed reparse.
        // None only from unset-language/timeout (neither happens here); if it did, next refresh re-parses window from scratch (self-healing)
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
        let capture_styles: Vec<Option<ratatui::style::Style>> = self
            .config
            .query
            .capture_names()
            .iter()
            .map(|name| self.palette.style_for_scope(name))
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

/// Compute the parse window byte range [start, end) for a viewport + margin.
/// Extends PARSE_MARGIN_LINES lines in each direction from the view, clamped to [0, len].
fn parse_area(text: &str, view: &[(usize, usize, usize)], margin_lines: usize) -> (usize, usize) {
    let len = text.len();
    if view.is_empty() || len == 0 {
        return (0, len);
    }
    let view_byte_start = view[0].1;
    let view_byte_end = view[view.len() - 1].2;

    // Walk backwards from view_byte_start counting margin_lines newlines.
    let area_start = {
        let mut remaining = margin_lines;
        let mut pos = view_byte_start;
        for (i, &b) in text.as_bytes()[..view_byte_start].iter().enumerate().rev() {
            if b == b'\n' {
                if remaining == 0 {
                    pos = i + 1;
                    break;
                }
                remaining -= 1;
            }
            if i == 0 {
                pos = 0;
            }
        }
        if remaining > 0 { 0 } else { pos }
    };

    // Walk forwards from view_byte_end counting margin_lines newlines.
    let area_end = {
        let mut remaining = margin_lines;
        let mut pos = len;
        let start = view_byte_end.min(len);
        for (off, &b) in text.as_bytes()[start..].iter().enumerate() {
            if b == b'\n' {
                if remaining == 0 {
                    pos = start + off + 1; // include the newline
                    break;
                }
                remaining -= 1;
            }
        }
        pos
    };

    (area_start, area_end.min(len))
}

/// Convert a byte offset within a line's text to a char column.
fn byte_to_char_col(line_text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(line_text.len());
    line_text[..clamped].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

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
        use crate::theme::Palette;
        let p = Palette::dark();
        assert_eq!(
            p.syntax.style_for_scope("keyword.function"),
            Some(ratatui::style::Style::default().fg(ratatui::style::Color::LightMagenta))
        );
        assert_eq!(
            p.syntax.style_for_scope("comment"),
            Some(ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray).add_modifier(ratatui::style::Modifier::ITALIC))
        );
        assert_eq!(p.syntax.style_for_scope("variable"), None);
        assert_eq!(p.syntax.style_for_scope("nonexistent.scope"), None);
        assert_eq!(p.syntax.style_for_scope("punctuation.bracket"), None);
        assert_eq!(p.syntax.style_for_scope("punctuation.delimiter"), None);
    }

    #[test]
    fn all_nine_grammars_construct() {
        for name in ["rust", "toml", "markdown", "json", "c", "bash", "python", "go", "lua"] {
            assert!(grammar(name).is_some(), "{name} missing");
            assert!(
                SyntaxState::build(name, "", &crate::theme::Palette::dark()).is_ok(),
                "{name} highlight config must construct"
            );
        }
    }

    #[test]
    fn python_go_lua_registered_and_highlighted() {
        assert_eq!(language_for(Some(Path::new("a.py"))), Some("python"));
        assert_eq!(language_for(Some(Path::new("stub.pyi"))), Some("python"));
        assert_eq!(language_for(Some(Path::new("m.GO"))), Some("go"));
        assert_eq!(language_for(Some(Path::new("init.lua"))), Some("lua"));
        let dark = crate::theme::Palette::dark();

        let mut s = SyntaxState::build("python", "def foo():\n    pass\n", &dark).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        let d = s.spans().iter().find(|f| f.line == 0 && f.col == 0).expect("python def span");
        assert_eq!(d.style.fg, Some(Color::LightMagenta), "python def is a keyword.function");

        let mut s = SyntaxState::build("go", "func main() {}\n", &dark).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        let f = s.spans().iter().find(|f| f.line == 0 && f.col == 0).expect("go func span");
        assert_eq!(f.style.fg, Some(Color::LightMagenta), "go func is a keyword.function");

        let mut s = SyntaxState::build("lua", "local x = 1\n", &dark).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        assert!(
            s.spans().iter().any(|sp| sp.style.fg == Some(Color::LightBlue)),
            "lua number 1 should get the number scope (LightBlue); got {:?}",
            s.spans().iter().map(|sp| (sp.col, sp.style.fg)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rust_keyword_spans_appear_on_their_line() {
        let mut s = SyntaxState::build("rust", "fn main() {}\n", &crate::theme::Palette::dark()).unwrap();
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
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
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

        let mut inc = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
        inc.refresh(&[], &view_lines(0, 1, inc.text()));

        let e_a = PendingEdit { start_byte: 0, end_byte: 0, insert: "pub ".to_string() };
        let e_b = PendingEdit { start_byte: 0, end_byte: 4, insert: String::new() };
        // "// café": 'é' ends at byte 8 (U+00E9, 2 bytes at bytes 6..8)
        let e_c = PendingEdit { start_byte: 8, end_byte: 8, insert: "x".to_string() };
        let final_text = "// caféx\nfn main() {}\n";

        let view = view_lines(0, 1, final_text);
        inc.refresh(&[e_a, e_b, e_c], &view);
        assert_eq!(inc.text(), final_text, "shadow after three edits");

        let mut fresh = SyntaxState::build("rust", final_text, &crate::theme::Palette::dark()).unwrap();
        fresh.refresh(&[], &view);

        let to_key = |spans: &[FrameSpan]| -> std::collections::BTreeSet<(usize, usize, usize, String)> {
            spans.iter().map(|s| (s.line, s.col, s.end_col, format!("{:?}", s.style.fg))).collect()
        };
        assert_eq!(to_key(inc.spans()), to_key(fresh.spans()), "incremental spans must match fresh rebuild");
    }

    #[test]
    fn unknown_language_is_an_error_not_a_panic() {
        assert!(SyntaxState::build("klingon", "x", &crate::theme::Palette::dark()).is_err());
    }

    #[test]
    fn frame_cache_skips_requery_same_view() {
        let src = "fn main() {}\n";
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
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
        let _ = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();
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

        let mut s = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();
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
        let mut s = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();
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
        let mut s = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();
        let view = view_lines(0, 59, &src);
        s.refresh(&[], &view);
        let view2 = view_lines(0, 59, &src);
        let t = std::time::Instant::now();
        s.refresh(&[], &view2);
        let elapsed = t.elapsed();
        println!("bench_syntax_warm_viewport_100k_lines: {elapsed:?}");
        assert!(elapsed.as_millis() < 2, "warm viewport too slow: {elapsed:?}");
    }

    /// Raw tree-sitter incremental parse, NO SyntaxState, 20-iteration loop.
    /// Documents the unwindowed library baseline: ~50 ms/iter stable on 100k-line Rust.
    /// plat uses windowed parse (view ± PARSE_MARGIN_LINES) which makes this irrelevant
    /// for normal edits; this bench is kept as a data point for the library itself.
    #[test]
    #[ignore]
    fn bench_raw_ts_incremental() {
        use std::time::Instant;
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let n = 100_000usize;
        let src = line.repeat(n);

        // One Parser, set_language ONCE — never called again.
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();

        // Initial full parse.
        let t = Instant::now();
        let mut tree = parser.parse(src.as_bytes(), None).unwrap();
        println!("bench_raw_ts_incremental: initial parse {:?}", t.elapsed());

        // Mid-file byte position: end of line 50000.
        let mid_byte = src.len() / 2;
        let mid_byte = src[mid_byte..].find('\n').map(|i| mid_byte + i + 1).unwrap_or(mid_byte);

        // Pre-compute the Point for mid_byte once (it won't change between iterations
        // because we insert+delete the same byte each time, keeping the source identical).
        let mid_row = src[..mid_byte].bytes().filter(|&b| b == b'\n').count();
        let mid_point = tree_sitter::Point::new(mid_row, 0);

        // Build the two alternating texts: original and with one '\n' inserted at mid_byte.
        let src_plus = format!("{}\n{}", &src[..mid_byte], &src[mid_byte..]);

        println!("bench_raw_ts_incremental: mid_byte={mid_byte} mid_row={mid_row}");

        // 20 iterations: insert '\n' then delete it — keeps source stable for fair comparison.
        for i in 0..20 {
            // --- INSERT '\n' at mid_byte ---
            let ie_ins = tree_sitter::InputEdit {
                start_byte:      mid_byte,
                old_end_byte:    mid_byte,
                new_end_byte:    mid_byte + 1,
                start_position:  mid_point,
                old_end_position: mid_point,
                new_end_position: tree_sitter::Point::new(mid_row + 1, 0),
            };
            tree.edit(&ie_ins);
            let t = Instant::now();
            tree = parser.parse(src_plus.as_bytes(), Some(&tree)).unwrap();
            let ins_time = t.elapsed();

            // --- DELETE that '\n' (revert to original) ---
            let ie_del = tree_sitter::InputEdit {
                start_byte:      mid_byte,
                old_end_byte:    mid_byte + 1,
                new_end_byte:    mid_byte,
                start_position:  mid_point,
                old_end_position: tree_sitter::Point::new(mid_row + 1, 0),
                new_end_position: mid_point,
            };
            tree.edit(&ie_del);
            let t = Instant::now();
            tree = parser.parse(src.as_bytes(), Some(&tree)).unwrap();
            let del_time = t.elapsed();

            println!("  iter {i:2}: insert={ins_time:?}  delete={del_time:?}");
        }
    }

    #[test]
    fn empty_lines_no_panic_and_sane_spans() {
        // "fn x(){}\n\nfn y(){}\n" — line 1 is empty, line 3 is the phantom final line.
        let src = "fn x(){}\n\nfn y(){}\n";
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
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

    /// Pins windowed-parse behavior: view near EOF on a 5100-line file, then an edit
    /// near file start while viewing the end — proves out-of-area tree.edits keep offsets right.
    #[test]
    fn windowed_parse_views_near_eof_and_out_of_area_edit() {
        let line = "pub fn fN() { let x: u32 = 0; }\n";
        let n = 5100usize;
        let src = line.repeat(n);

        let mut s = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();

        // View: last 10 lines (outside the initial parse window for PARSE_MARGIN_LINES=2000).
        let view_end = view_lines(5090, 5099, &src);
        s.refresh(&[], &view_end);

        // Spans must exist in the viewed window.
        assert!(s.spans().iter().any(|sp| sp.line >= 5090),
            "expected spans in viewed window near EOF");
        // No spans for lines far off-screen (parse window covers only the last ~2060 lines).
        assert!(s.spans().iter().all(|sp| sp.line >= 5090),
            "expected no spans for off-screen lines");

        // Edit near file start (line 0) while viewing the end.
        // Out-of-area tree.edit must keep byte offsets correct so viewed spans stay valid.
        let insert = "// \n";
        let edit = PendingEdit { start_byte: 0, end_byte: 0, insert: insert.to_string() };
        let new_src = format!("{insert}{src}");
        // The inserted line shifts everything down by 1; viewed window is now 5091..5100.
        let new_view = view_lines(5091, 5100, &new_src);
        s.refresh(&[edit], &new_view);

        assert!(s.spans().iter().any(|sp| sp.line >= 5091),
            "expected spans after out-of-area edit near file start");
    }

    /// O(window) claim: edit cost at 100k and 400k lines must be similarly fast.
    #[test]
    #[ignore]
    fn bench_syntax_edit_400k_lines_ow_window() {
        use std::time::Instant;
        let line = "pub fn fN() { let x: u32 = 0; }\n";

        for &count in &[100_000usize, 400_000] {
            let src = line.repeat(count);
            let mid = { let m = src.len() / 2; src[m..].find('\n').map(|i| m+i+1).unwrap_or(m) };
            let mid_line = mid / line.len();
            let mut s = SyntaxState::build("rust", &src, &crate::theme::Palette::dark()).unwrap();
            s.refresh(&[], &view_lines(mid_line, mid_line + 59, &src));
            let edit = PendingEdit { start_byte: mid, end_byte: mid, insert: "\n".to_string() };
            let new_src = format!("{}\n{}", &src[..mid], &src[mid..]);
            let t = Instant::now();
            s.refresh(&[edit], &view_lines(mid_line, mid_line + 59, &new_src));
            let elapsed = t.elapsed();
            println!("bench_syntax_edit_ow_window {count}k lines: {elapsed:?}");
            // 100k: ≤10ms; 400k allows 20ms (string alloc overhead for 13MB source).
            let limit = if count <= 100_000 { 10 } else { 20 };
            assert!(elapsed.as_millis() < limit, "{count}k edit too slow: {elapsed:?}");
        }
    }

    // --- symbol_chain (#56 breadcrumb) ---

    #[test]
    fn symbol_chain_rust_impl_method_top_and_cap() {
        let src = "impl App {\n    fn handle_command(&mut self) {\n        let x = 1;\n    }\n}\n";
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        // caret inside the method body (line 2, before `let`)
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(2, 12)),
            vec!["impl App", "fn handle_command"], "outer->inner labels");
        // on the impl header itself
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(0, 0)), vec!["impl App"]);
    }

    #[test]
    fn symbol_chain_top_level_is_empty() {
        let src = "fn alpha() {}\nlet top = 1;\n";
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(1, 4)), Vec::<String>::new());
    }

    #[test]
    fn symbol_chain_python_class_method() {
        let src = "class Foo:\n    def bar(self):\n        pass\n";
        let mut s = SyntaxState::build("python", src, &crate::theme::Palette::dark()).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(2, 8)),
            vec!["class Foo", "fn bar"]);
    }

    #[test]
    fn symbol_chain_caps_at_three_innermost() {
        let src = "fn a() {\n fn b() {\n  fn c() {\n   fn d() {\n    let x = 1;\n   }\n  }\n }\n}\n";
        let mut s = SyntaxState::build("rust", src, &crate::theme::Palette::dark()).unwrap();
        let view = view_lines(0, 0, s.text());
        s.refresh(&[], &view);
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(4, 4)),
            vec!["fn b", "fn c", "fn d"], "keeps the innermost 3");
    }

    #[test]
    fn symbol_chain_no_tree_is_empty() {
        let s = SyntaxState::build("rust", "fn a() {}\n", &crate::theme::Palette::dark()).unwrap();
        assert_eq!(s.symbol_chain(tree_sitter::Point::new(0, 3)), Vec::<String>::new());
    }
}
