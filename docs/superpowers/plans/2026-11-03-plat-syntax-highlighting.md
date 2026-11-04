# Syntax Highlighting (tree-sitter) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Visible-buffer syntax highlighting via tree-sitter for six languages (rust, toml, markdown, json, c, bash), incremental across every mutation path, viewport-windowed query cost, fixed color palette, `[editor] syntax` config switch.

**Architecture:** A new `src/syntax.rs` owns everything tree-sitter-facing: a `SyntaxState` per `Document` holding a shadow text `String` + `tree_sitter_highlight::Highlighter`. All rope mutations already funnel through `Document::edit`, which appends byte-level `PendingEdit`s; `App::ensure_syntax` (called from the run loop before `draw`) builds/flushes/queries visible rows only; `render.rs` reads precomputed frame spans. A build error degrades to plain text + one status note.

**Tech Stack:** Rust (edition 2024), ratatui 0.30, ropey 1.6; new deps: `tree-sitter`, `tree-sitter-highlight`, and grammar crates `tree-sitter-rust`, `tree-sitter-toml`, `tree-sitter-md`, `tree-sitter-json`, `tree-sitter-c`, `tree-sitter-bash`.

**Spec:** `docs/superpowers/specs/2026-11-03-plat-syntax-highlighting-design.md` (read it alongside this plan)

## Global Constraints

- New dependencies: the seven tree-sitter crates listed above — nothing else. Grammar + core compile C with `cc` (already required); no Zig. Binary stays < 50 MB.
- `[editor] syntax = true` default; when false (or no path / unknown extension) NO tree, shadow String, or edit accumulator is ever created — plain-text cost must be literally zero.
- Highlighting never breaks editing: any grammar/query error → `syntax = None`, one status note `syntax: {short}`, editing/searching unaffected.
- Palette values exactly per the spec table (LightMagenta keyword, LightCyan function, Yellow type, Green string, LightBlue number/constant, DarkGrey+italic comment, LightYellow property/field, Red error, default otherwise).
- `Document::edit` (src/document.rs:110) is the ONLY rope mutation point (verified — incl. undo/redo, replace_all batches); the edit hook lives there. `App::reload_doc` (app.rs:853) replaces ropes wholesale → must reset `syntax = None`.
- Perf gates (release, `#[ignore]`, 100k-line rust file): first build ≤ 150 ms; one edit flush+reparse ≤ 5 ms; viewport query ≤ 2 ms. Report numbers + binary size in README acceptance.
- Test gates: `cargo test` green (baseline 175 unit + 9 flow at plan time), `cargo build` warning-free.

## Review Focus

1. **Incremental desync through batch paths** — a `replace_all`/undo batch emits MANY pending edits; flushing them out of order or missing one silently corrupts the tree, showing wrong colors far from the edit. (Task 3: replay-consistency test over undo batch + replace_all.)
2. **Multibyte text offset confusion** — tree-sitter is byte-based, cells are char-based; a line with `é`/emoji must still highlight the right COLUMNS. (Task 2: span-column test on a multibyte line.)
3. **Zero cost when disabled** — plain-text buffer or `syntax = false`: no `PendingEdit`s accumulate, no shadow string, no query (no memory/time leak of the feature into non-code files). (Tasks 3/5.)
4. **Grammar/ABI mismatch at build** — grammar crates must link against the same tree-sitter ABI; a mismatch panics or errors at `HighlightConfiguration::new`. (Task 1: a test constructing all six grammars.)
5. **100k-line responsiveness** — first open stalls visible typing; keystrokes feel laggy. (Task 7: the ignored perf gates + reported numbers.)

---

### Task 1: deps + pure pieces (`language_for`, palette, `style_for_scope`)

**Files:** Modify `Cargo.toml`; Create `src/syntax.rs`, `src/lib.rs` (`pub mod syntax;`); tests in-module.

**Interfaces:**
- Produces: `pub fn language_for(path: Option<&std::path::Path>) -> Option<&'static str>` (ext map: rs→"rust", toml→"toml", md→"markdown", json→"json", c|h→"c", sh|bash→"bash"; case-insensitive); `pub fn style_for_scope(scope: &str) -> Option<ratatui::style::Style>` (longest-prefix table per the spec palette); `pub struct LanguageCtor(pub &'static tree_sitter::Language)` — `pub fn grammar(lang: &'static str) -> Option<tree_sitter::Language>` mapping the six names to grammar crates.

- [ ] **Step 1:** `cargo add tree-sitter tree-sitter-highlight tree-sitter-rust tree-sitter-toml tree-sitter-md tree-sitter-json tree-sitter-c tree-sitter-bash` — then check `cargo tree -d` for duplicate tree-sitter versions; if a grammar crate pins an older tree-sitter, pick versions that unify (this is Review Focus 4 groundwork).
- [ ] **Step 2: failing tests**
```rust
#[test] fn extension_map_is_case_insensitive() {
    assert_eq!(language_for(Some(std::path::Path::new("a.RS"))), Some("rust"));
    assert_eq!(language_for(Some(std::path::Path::new("x/README.MD"))), Some("markdown"));
    assert_eq!(language_for(Some(std::path::Path::new("k.h"))), Some("c"));
    assert_eq!(language_for(Some(std::path::Path::new("notes.txt"))), None);
    assert_eq!(language_for(None), None);
}
#[test] fn palette_scopes_resolve() {
    assert_eq!(style_for_scope("keyword.function"), Some(Style::default().fg(Color::LightMagenta)));
    assert_eq!(style_for_scope("comment"), /* DarkGrey + ITALIC per spec table */);
    assert_eq!(style_for_scope("variable"), None);          // default — no style
    assert_eq!(style_for_scope("nonexistent.scope"), None);
}
#[test] fn all_six_grammars_construct() {  // Review Focus 4
    for name in ["rust","toml","markdown","json","c","bash"] {
        assert!(grammar(name).is_some(), "{name} missing");
    }
}
```
- [ ] **Step 3:** run `cargo test syntax::` → fail to compile; implement; pass.
- [ ] **Step 4:** full `cargo test` green; `cargo build --release` once and record binary size (`ls -s target/release/plat`).
- [ ] **Step 5:** Commit `syntax: language map, palette table, grammar registry`

### Task 2: `SyntaxState` — build, apply edits, viewport query

**Files:** Create `src/syntax.rs` (state part); tests in-module.

**Interfaces:**
- Consumes: `grammar()`, `style_for_scope` (Task 1).
- Produces (used by Tasks 3–4):
  - `pub struct PendingEdit { pub start_byte: usize, pub end_byte: usize, pub insert: String }` (defined in syntax.rs; byte offsets, half-open old range)
  - `pub struct FrameSpan { pub line: usize, pub col: usize, pub end_col: usize, pub style: ratatui::style::Style }` (char cols from line start)
  - `impl SyntaxState`: `pub fn build(lang: &'static str, text: &str) -> Result<SyntaxState, String>`; `pub fn refresh(&mut self, edits: &[PendingEdit], view: &[(usize, usize, usize)] /* (line, byte_start, byte_end) inclusive ranges into the text */)`; `pub fn spans(&self) -> &[FrameSpan]`; `pub fn text(&self) -> &str` (the shadow).
  - `refresh` also SKIPS re-query when `edits.is_empty()` and the view slice equals the previous one (frame cache).

- [ ] **Step 1: failing tests**
```rust
#[test] fn rust_keyword_spans_appear_on_their_line() {
    let mut s = SyntaxState::build("rust", "fn main() {}\n").unwrap();
    s.refresh(&[], &view_lines(0, 0, s.text()));   // helper: (line, byte range) pairs
    let fn_span = s.spans().iter().find(|f| f.line == 0 && f.col == 0).unwrap();
    assert_eq!(fn_span.style.fg, Some(Color::LightMagenta)); // keyword "fn"
}
#[test] fn spans_use_char_cols_on_multibyte_lines() {   // Review Focus 2
    let mut s = SyntaxState::build("rust", "// é\nfn main() {}\n").unwrap();
    // a multibyte COMMENT line above must not shift line 0's "fn" span:
    s.refresh(&[], &view_lines(0, 1, s.text()));
    let l0 = s.spans().iter().find(|f| f.line == 0 && f.style.fg == Some(Color::LightMagenta)).unwrap();
    assert_eq!(l0.col, 0); assert_eq!(l0.end_col, 2);
}
#[test] fn incremental_matches_fresh_rebuild() {        // Review Focus 1 core (single-edit case)
    // script: build("rust", src); apply edits [(insert "pub " at 0, delete 3 bytes at 5, insert "x" mid-multibyte)];
    // then compare spans(view) to a SyntaxState::build of the final text → equal (style,line,col,end) sets
}
#[test] fn unknown_language_is_an_error_not_a_panic() {
    assert!(SyntaxState::build("klingon", "x").is_err());
}
```
- [ ] **Step 2:** verify fail; **Step 3: implement** — `build`: `HighlightConfiguration::new(lang, HIGHLIGHT_QUERY, "", "")` per grammar crate (each exposes its query const; for `tree-sitter-md` use its markdown-highlight const; if the crate's query rejects `Canceled`/injections, embed upstream `highlights.scm` as a `const &str` — record which path in the report). Register palette entries via `Highlighter::set_style`. Shadow `String` = text. `refresh`: splice shadow with the byte ranges (same order as `edits`), build `tree_sitter::InputEdit`s from the shadow (compute row/col BEFORE and AFTER each splice from the shadow itself — it mirrors the rope, so points are exact), `update(&edits)` + `parse`, then `highlight(shadow, ranges)`; map byte segments to `FrameSpan`s char-wise per view line. `language != found` → Err (grammar() None).
- [ ] **Step 4:** `cargo test syntax::` green + full suite. **Step 5:** Commit `syntax: SyntaxState with shadow-text edits and viewport query`

### Task 3: Document hook + `App::ensure_syntax` (build/flush/query lifecycle)

**Files:** Modify `src/document.rs` (fields + `edit`), `src/app.rs` (`ensure_syntax`, `new_with_config`), `src/lib.rs` (run loop calls it before `terminal.draw`), `tests/flow.rs` (press-loop equivalent calls it). Tests in both src files.

**Interfaces:**
- Consumes: `SyntaxState` (Task 2), `language_for`, `config.editor` (Task 5 lands the config field; until then use `true`/`Config::defaults()`-style placeholder constant `SYNTAX_ENABLED_BY_DEFAULT: bool = true` — Task 5 replaces it).
- Produces: `Document.syntax: Option<SyntaxState>`, `Document.pending_ts: Vec<PendingEdit>` (default None/empty in `empty()`/`open()`); `Document::edit` pushes a `PendingEdit { start_byte: rope.char_to_byte(at), end_byte: rope.char_to_byte(at + remove_len), insert: insert.to_string() }` ONLY when `syntax.is_some()` (Review Focus 3 gate); `pub fn App::ensure_syntax(&mut self, view_rows: usize)`; `App::reload_doc` sets `docs[idx].syntax = None; docs[idx].pending_ts.clear()`.

- [ ] **Step 1: failing tests** (document.rs)
```rust
#[test] fn edit_accumulates_only_when_syntax_present() {  // Review Focus 3
    let mut d = Document::empty();
    d.edit(0, "fn x(){}", 0);
    assert!(d.pending_ts.is_empty());          // plain buffer: zero-cost, nothing collected
    d.syntax = Some(SyntaxState::build("rust", "fn x(){}").unwrap());
    d.pending_ts.clear();                      // build consumed/adopted the current text
    d.edit(3, " ", 0);
    assert_eq!(d.pending_ts.len(), 1);
    d.undo();                                  // undo ALSO feeds the accumulator  // Review Focus 1
    assert_eq!(d.pending_ts.len(), 2);
}
#[test] fn batch_paths_emit_every_edit() {     // Review Focus 1: replace_all_from + undo batch
    // 3-match replace_all → pending_ts has one entry per rope splice, in order;
    // then undo the whole batch → equal count of reverse splices appended
}
```
(app.rs) `ensure_syntax` builds for the active doc when path maps + enabled (status note on Err: `syntax: {e}` once), else no-op; calls `state.refresh(&doc.pending_ts, view)` then `doc.pending_ts.clear()`. View rows: byte ranges via rope (`line_to_char`/`char_to_byte`) for `scroll_row..min(scroll_row+view_rows, len_lines)`.
- [ ] **Step 2:** verify fail; Step 3: implement (wire `run()` loop: `app.ensure_syntax(terminal.size().unwrap_or((80,24)).1.saturating_sub(2))` before draw; flow `press()` calls `app.ensure_syntax(22)` after each dispatch).
- [ ] **Step 4:** `cargo test` green. Step 5: Commit `app+document: syntax lifecycle hooked into the edit choke point and run loop`

### Task 4: render — paint frame spans per cell

**Files:** Modify `src/render.rs` (the char-paint loop at ~lines 107–113); tests in-module.

**Interfaces:** Consumes `doc.syntax` frame spans (Tasks 2–3). Produces nothing new for later tasks.

- [ ] **Step 1: failing test** (render.rs, reuse existing `doc_app`/buffer helpers)
```rust
#[test] fn keyword_cells_carry_the_palette_color() {
    let mut app = doc_app_named("t.rs", "fn main() {}\n");   // helper: named temp file + open
    app.ensure_syntax(24);
    draw_into_buffer(&app);  // existing buffer-drawing path
    assert_eq!(buf[(ed_x + gutter, 0)].style().fg, Some(Color::LightMagenta)); // 'f' of "fn"
    // same cell under an open Find prompt matching "fn": underline added ON TOP, fg kept
}
#[test] fn plain_text_document_has_no_syntax_styles() {      // Review Focus 3
    // doc_app("plain.txt") → draw: every visible cell style == default
}
```
- [ ] **Step 2:** verify fail; Step 3: implement — right after the per-line char-paint loop (before the selection block), for `doc.syntax` frame spans of `line_idx`: `for col in span.col..span.end_col` → same visibility math as the char loop → `buf[..].set_style(span.style)`. Selection/search blocks below it are untouched (they win by painting later).
- [ ] **Step 4:** `cargo test render::` + full suite green. Step 5: Commit `render: syntax frame spans painted under selection/search layers`

### Task 5: `[editor] syntax` config field

**Files:** Modify `src/config.rs` (EditorCfg/RawEditor/fields list/seed), `src/app.rs` (replace the Task-3 placeholder with `config.editor.syntax`), `src/document.rs` (drop placeholder const). Tests in config.rs + app.rs.

**Interfaces:** Produces `EditorCfg.syntax: bool` (default **true**); `"syntax"` joins `all_editor_fields` (did-you-mean list). `ensure_syntax` no-ops when `!app.config.editor.syntax`.

- [ ] **Step 1: failing tests:** `from_str("[editor]\nsyntax = false")` → `cfg.editor.syntax == false`; default true; unknown field `syntaxx` → suggestion contains `syntax`. App test: syntax=false + open .rs file → `doc.syntax.is_none()` after `ensure_syntax`, and `edit` accumulates nothing.
- [ ] **Step 2:** fail; Step 3: implement (5-point config touch is the established pattern from line_numbers/key_debug — copy it).
- [ ] **Step 4:** full suite. Step 5: Commit `config: [editor] syntax switch, default on`

### Task 6: reload/undo survival — integration (flow)

**Files:** Modify `tests/flow.rs`; verify-only for app.rs reload path.

- [ ] **Step 1: failing test**
```rust
#[test] fn syntax_survives_edit_undo_and_reload() {
    // temp .rs with "fn a(){}"; flow: type_text an edit, undo, assert keyword cell still LightMagenta;
    // fs::write a NEW content on disk, drive handle_external_change path (clean doc → silent reload),
    // assert colors follow the NEW text (old buffer's spans gone; syntax rebuilt)
}
```
- [ ] **Step 2:** implement whatever the test exposes (reload reset in Task 3 should cover it; if reload re-opens through `Document::open` note in report).
- [ ] **Step 3:** full suite. Step 4: Commit `flow: syntax survives edit/undo/reload`

### Task 7: perf gates, README, PLAN, smoke

**Files:** Modify `src/syntax.rs` (`#[ignore]` bench tests like document.rs's), `README.md`, `PLAN.md`.

- [ ] **Step 1:** ignored release benches on 100k-line rust: `SyntaxState::build` ≤150ms; 1 edit `refresh` ≤5ms; warm viewport `refresh` ≤2ms. Run `cargo test --release -- --ignored syntax` and record actual numbers.
- [ ] **Step 2:** `cargo build --release` binary size recorded (must stay <50MB).
- [ ] **Step 3:** README: new "Syntax highlighting" section (languages table, `[editor] syntax`, palette note incl. "themes come in #6"), Acceptance (syntax) section: test counts, the 5 Review Focus behaviors + named tests, perf numbers, binary size. PLAN.md: work order #3 ✅ line.
- [ ] **Step 4:** pty smoke (reuse config_smoke.py pattern): open a .rs file, type + Ctrl-Q, exit 0, no panic text, status has no "syntax:" note. Record transcript key lines.
- [ ] **Step 5:** final `cargo test` + `cargo test --test flow`; commit `docs+benches: syntax acceptance`.

---

## Self-review notes

- Spec coverage: detection+config → T1/T5; state/shadow/edits/query → T2/T3; render layering → T4; palette → T1; errors → T2/T3 (status note T3); perf → T1(size)/T7; testing list → T2–T7. Reload reset → T3 (impl) + T6 (test).
- Review Focus pinned: 1 → T3 `batch_paths_emit_every_edit` + T2 `incremental_matches_fresh_rebuild`; 2 → T2 `spans_use_char_cols_on_multibyte_lines`; 3 → T3 `edit_accumulates_only_when_syntax_present` + T4 `plain_text_document_has_no_syntax_styles` + T5 app test; 4 → T1 `all_six_grammars_construct` + `cargo tree -d`; 5 → T7 benches.
- Names pinned across tasks: `PendingEdit{start_byte,end_byte,insert}`, `FrameSpan{line,col,end_col,style}`, `SyntaxState::{build,refresh,spans,text}`, `language_for`, `grammar`, `style_for_scope`, `App::ensure_syntax(view_rows)`, fields `Document.syntax`, `Document.pending_ts`, `EditorCfg.syntax`.
