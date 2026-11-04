# Syntax Highlighting (tree-sitter) — Design

Date: 2026-11-03 · Work order #3 in PLAN.md · Status: approved in conversation, awaiting written-spec review

## Goal

Visible-buffer syntax highlighting with tree-sitter: six bundled languages, incremental reparse on every edit (including undo/redo and external reload), viewport-only query cost per frame, and a fixed built-in color palette. Highlighting must never break editing.

## Non-goals

- Themes and user color config — work order #6 replaces the built-in palette table; the scope names defined here are its interface.
- Fold indicators, indentation guides, "smart" pairs/textobjects from the parse tree, a syntax-tree debug view, language injections (code blocks inside Markdown stay one color family).
- Filetype overrides in config (`filetype = "rust"` per buffer): YAGNI; the extension map below is the whole mapping.
- Status-bar language display.

## Dependencies

- `tree-sitter` (runtime) and `tree-sitter-highlight` (queries + style-scope machinery), plus six grammar crates: `tree-sitter-rust`, `tree-sitter-toml`, `tree-sitter-md` (Markdown; fall back to embedding the grammar's official `highlights.scm` if the crate doesn't ship queries), `tree-sitter-json`, `tree-sitter-c`, `tree-sitter-bash`.
- Build note: grammar crates and tree-sitter's core compile C with `cc` — the same tool Rust linking already requires on this platform. No new toolchain requirement, no Zig. Binary grows ≈4–6 MB; cap remains 50 MB.
- No other new dependencies.

## Language detection

`fn language_for(path: &Path) -> Option<&'static str>` (case-insensitive extension):

| ext | language |
|---|---|
| `rs` | rust |
| `toml` | toml |
| `md` | markdown |
| `json` | json |
| `c`, `h` | c |
| `sh`, `bash` | bash |

No path (scratch buffer), unknown extension, or `syntax = false` → plain text, no tree built.

## Config

`[editor] syntax = true` (default). It joins the existing seeded options; `false` means no tree is ever built and rendering is exactly today's. Nothing else new: colors come from the built-in palette until themes (#6).

## Architecture

One new module, `src/syntax.rs`, holding everything tree-sitter-facing:

- **`SyntaxState`** — per buffer, stored as `Option<SyntaxState>` on `Document` (next to `search_matches`); `None` = plain text. Constructed lazily on first draw of a buffer whose path maps to a language. `Document::reload` (external-change path) sets it to `None`; next draw rebuilds.
- **Edit capture** — `Document::edit` (document.rs:110) is the single choke point for *every* rope mutation, undo/redo included (verified: no `rope.edit` calls anywhere else). It appends to a small `pending_edits: Vec<tree-sitter::InputEdit>` accumulator that `SyntaxState` flushes into the highlighter before each query. Char→byte conversion via ropey (`char_to_byte`). The accumulator decouples the rope model from tree-sitter types: `Document` stays constructible without the syntax feature (field is inert when `None`).
- **Text access** — `tree_sitter_highlight::Highlighter::highlight` requires one contiguous `&str`, which ropey cannot provide per frame. `SyntaxState` therefore keeps a **shadow `String`** of the buffer text, mutated with the very same edits (byte offsets are already computed for the `InputEdit`s — one byte-level splice, same numbers). Cost: one text copy (a few MB for 100k lines) and a memmove per mid-file edit (measured in the perf gate).
- **Query** — each draw, for the current buffer: flush edits into the shadow string + highlighter, then `Highlighter::highlight(shadow, &ranges)` with `ranges` = byte spans of the **visible rows** only. A viewport-sized window on a 100k-line file costs viewport-sized styling work; the maintained parse tree covers the whole file.

### Rendering and layering

`render.rs` gains one helper: for each visible row, produce styled spans from `SyntaxState`, then apply existing layers on top, in today's precedence:

1. syntax spans (base style per scope, default style where unstyled),
2. search-match highlight (unchanged — wins over syntax),
3. current-line/cursor behavior unchanged.

Unstyled scopes keep the terminal default — the palette is additive, never an erase.

### Palette (fixed, `syntax.rs` const table)

tree-sitter-highlight scope → ratatui `Style`, ~12 entries:

| scope | style |
|---|---|
| `keyword*` (prefix match) | LightMagenta |
| `function`, `function.method` | LightCyan |
| `type`, `type.builtin` | Yellow |
| `string*`, `char` | Green |
| `number`, `constant` | LightBlue |
| `comment` | DarkGrey (italic) |
| `punctuation.bracket`, `punctuation.delimiter` | default fg (no color) |
| `variable`, `variable.parameter` | default |
| `property`, `field` | LightYellow |
| `operator` | default |
| `embedded`, `conceal` | default |
| `error` | Red |

The table is applied through `Highlighter::set_style` prefix registration (registering `keyword` covers `keyword.function` etc. — hierarchical scope matching comes free); `style_for_scope(scope: &str) -> Option<Style>` is the pure helper underneath, for tests. Task 6 (themes) swaps this table; nothing else changes.

## Error handling

Highlighting failures are display-layer-only. At state construction or any draw: grammar/query/`Canceled` errors → set `None`, set status line once: `syntax: {short}` (reuses the existing status mechanism, same style as save-failure notes). The buffer remains fully editable/searchable — the search layer and everything else never see a syntax error. A per-frame query cost outlier is bounded by the viewport windowing, not retried.

## Performance acceptance

`#[ignore]`-gated release benchmarks (same pattern as the search 100k gate), against a 100k-line generated Rust file:

- first build (full parse + query warm frame) ≤ 150 ms
- one edit (flush + incremental reparse) ≤ 5 ms
- one viewport query (60 visible rows) after warmup ≤ 2 ms

Report the numbers in README acceptance (like search's 9.7/54/31 ms line). Binary size after six grammars recorded too; must stay < 50 MB.

## Testing

- Unit (`syntax.rs`): extension map (incl. `H`, `README.MD` case-insensitivity, unknown ext, `None` path); `style_for_scope` longest-prefix; edit-replay consistency — scripted edits (insert/delete/undo-batch) applied through `Document::edit`, then compare queried captures against a from-scratch rebuild (must agree — this is the incremental-correctness pin).
- Unit (`document.rs`): `pending_edits` accumulate on every mutation path incl. undo/redo and `replace_all_from` batch; cleared on flush.
- Unit (`render.rs`): fixture snippet → assert styled cells in the `Buffer` (keyword colored, comment colored, plain-text buffer unchanged when `syntax = false` or no extension); search match still overrides syntax color on an overlapping span.
- Flow (`tests/flow.rs`): open a `.rs` temp file, edit, undo — colors survive both (via buffer cell style assertions).
- Config chain: `[editor] syntax = false` → no `SyntaxState` ever appears.

## Decisions pinned

- Approach A: `tree-sitter-highlight` crate + official queries (not raw queries, not regex).
- Six grammars: rust, toml, markdown, json, c (+h), bash.
- State lives on `Document` as `Option<SyntaxState>`; edits captured at the `Document::edit` choke point via a pending-edits accumulator; viewport-only querying; fixed palette table as the theme seam; syntax errors degrade to plain text with one status note, never block editing.
