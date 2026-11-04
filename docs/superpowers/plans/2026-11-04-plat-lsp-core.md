# LSP Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** diagnostics + completion + goto-definition over real LSP servers — hand-rolled JSON-RPC/stdio core + `lsp-types`, reader thread per server, nothing ever blocks typing.

**Architecture:** `src/lsp/` owns everything LSP: a stdio framer, a `LspHub` in `App` holding one server per (language, project-root) with a reader thread pushing `LspEvent`s into an `mpsc` the run loop drains each poll, and full-text debounced document sync. UX layers (diagnostics gutter badges, completion menu, jump stack) live in render/app like every other feature. Tests use `PLAT_LSP_MOCK=1`: the plat binary itself becomes a deterministic stdio language server.

**Tech Stack:** Rust edition 2024; new deps: `lsp-types`, `serde_json` (serde already present), plus existing ropey/crossterm/ratatui. No async runtime.

**Spec:** `docs/superpowers/specs/2026-11-04-plat-lsp-core-design.md` (binding — read it with this plan)

## Global Constraints

- Deps: only `lsp-types` + `serde_json` added. No tokio, no URL crates, no lsp client crates.
- Key path unchanged: no LSP call may block/wait on the main thread; all reads on reader threads; handshake async ("attaching" state).
- Nothing LSP can break plain editing: no server / dead server / garbage stream → one status note per language, editor behaves exactly as before.
- Strict config schema extended with `[lsp]` table: value = string (argv, whitespace-split) or `false`.
- UTF-16 everywhere at the LSP boundary; char offsets internally. Line-bounded conversion only.
- Perf gates (release, `#[ignore]`): keystroke dispatch ≤ +10% vs pre-LSP baseline; 100k-line debounced full-sync single write (expect < 10 ms, off key path); 1000-diagnostic drain ≤ 1 ms; binary < 50 MB.
- Test gates: `cargo test` green (baseline **204 unit + 12 flow** at plan time), warning-free.
- `Document.edit/undo/redo/replace_all_from` are the four mutation choke points — the edit-generation counter MUST bump in all four (same sites that push PendingEdits; do NOT add a fifth path).

## Review Focus

1. **Never-block invariant** — a hung/silent server must leave typing, saving, quitting at full speed; handshake and requests are fire-and-forget with responses drained like events. (Task 3 test: request to mock, kill it mid-flight, app keeps processing keys.)
2. **Frame desync** — split/partial reads, multiple messages per read, oversized headers: one malformed byte stream must not shift all later messages or crash. (Task 1 tests.)
3. **UTF-16 offset math** — emoji/CJK lines: diagnostic badges, completion `textEdit` ranges, goto-def positions must land on the right cells. Off-by-one there is silent corruption. (Tasks 4/5/6 tests.)
4. **Stale-response races** — a completion response for text that changed since the request must never mutate the buffer; goto-def intentionally applies after edits. (Task 6/7 tests on the generation rule.)
5. **Zero LSP footprint when disabled/absent** — no server binary, `[lsp] toml = false`: no spawn, no state, byte-identical rendering (gutter, status, flow baselines). (Tasks 2/5 tests.)

---

### Task 1: deps + `src/lsp/transport.rs` (framing only)

**Files:** `Cargo.toml` (`cargo add lsp-types serde_json`); create `src/lsp/mod.rs` (empty module shell + `pub mod transport;`), `src/lsp/transport.rs`; `src/lib.rs` (`pub mod lsp;`). Tests in-module.

**Interfaces (consumed by every later task):**
- `pub fn write_message(w: &mut impl std::io::Write, msg: &serde_json::Value) -> std::io::Result<()>` — one `Content-Length: N\r\n\r\n`-framed JSON line.
- `pub struct Frmr { buf: Vec<u8>, overflow: Option<...> }` with `pub fn read_message<R: std::io::BufRead>(&mut self, r: &mut R) -> std::io::Result<Option<serde_json::Value>>` — handles partial reads (returns None at clean EOF), buffers across calls, emits successive messages; malformed (non-numeric Content-Length, header > 64 KB, content shorter than header at EOF) → `Ok(None)` after which the stream is poisoned (subsequent calls None).

**Steps:** 1. Failing tests: round-trip write→read (3 messages of mixed unicode); byte-at-a-time reader (every byte split lands correctly — loop feeding one byte through a Cursor-backed BufRead); garbage header `Content-Length: abc` → None; header `Content-Length: 99999999999` (>64KB cap) → None. 2. RED. 3. Implement (no lsp-types usage yet — transport speaks `serde_json::Value`). 4. `cargo test lsp::` + full suite green. 5. Commit `lsp: Content-Length framer with poisoned-stream semantics`.

### Task 2: `[lsp]` config section

**Files:** `src/config.rs`; `src/lsp/mod.rs` (defaults table + `command_for`). Tests in both.

**Interfaces:** `Config.lsp: LspCfg` with `pub fn command_for(&self, lang: &str) -> Option<Vec<String>>`. Raw shape: `Option<BTreeMap<String, LspValue>>` where `LspValue` is untagged string→Command(String) | false→Disabled. Defaults table (spec): rust→`["rust-analyzer"]`, c→`["clangd"]`, toml→`["taplo","lsp","stdio"]`, markdown→`["marksman"]`, bash→`["bash-language-server","start"]`, json→`["vscode-json-language-server","--stdio"]`. Override string → whitespace-split argv; `false` → None; unknown lang key with no default → Command usable if configured (e.g. `[lsp] python = "pyright"`) — allowed, any key. Empty string command `""` → Disabled-equivalent (documented).

**Steps:** failing tests: default table for all six langs; override splits `"taplo lsp stdio"`; `toml = false` → None; `""` → None; strict unknown *section* still errors (existing); `PLAT_CONFIG` file with `[lsp] rust = "my-ra --flag"` round-trips. RED → implement → full suite → commit `config: [lsp] server section with defaults + overrides`.

### Task 3: `LspHub` + reader threads + `PLAT_LSP_MOCK` server

**Files:** `src/lsp/mod.rs` (hub), `src/lsp/mock.rs`, `src/main.rs`/`src/lib.rs` (env switch: `if std::env::var("PLAT_LSP_MOCK").is_ok() { lsp::mock::serve(); exit(0); }` — pattern-match the existing `PLAT_PANIC` precedent), `tests/lsp_flow.rs` (new integration file). Tests: in-module (pure bits) + integration (real mock subprocess via `env!("CARGO_BIN_EXE_plat")`).

**Interfaces (pinned — Tasks 4–7 build on these):**
```rust
pub enum LspEvent {
    Ready { lang: &'static str, root: PathBuf },
    Response { id: u64, result: serde_json::Value },
    Diagnostics { path: PathBuf, diags: Vec<lsp_types::Diagnostic> },
    Died { lang: &'static str },
}
pub struct LspHub {
    // per (lang, root): child handle + stdin writer + next id + Attaching|Ready|Dead + queued msgs
    pub fn ensure(&mut self, lang: &'static str, root: &Path, cfg_cmd: &[String]) -> bool; // spawn + initialize handshake; false = spawn failed (note-once handled by caller)
    pub fn request(&mut self, lang: &str, root: &Path, method: &str, params: serde_json::Value) -> Option<u64>; // queued until Ready
    pub fn notify(&mut self, lang: &str, root: &Path, method: &str, params: serde_json::Value);
    pub fn take_events(&mut self) -> Vec<LspEvent>; // drain the mpsc
    pub fn shutdown_all(&mut self); // shutdown+exit, kill after 300ms
}
```
- App gets `pub lsp: LspHub`; run loop + flow `press()` call a pump (Task 5 wires consumption; this task only drains-and-discards to prove plumbing).
- Mock (`serve()`): reads framed requests; replies `initialize` (capabilities: full sync); `initialized` → sends `publishDiagnostics` for the opened doc (1 error line1 "mock error", 1 warning line2 "mock warning"); `textDocument/completion` → 3 items (plain `alpha_full`; snippet `beta_x(${1:arg})`; one with `textEdit` replacing chars -3..0 with `gamma_edit`); `textDocument/definition` → single Location (uri of request doc, range 0,0..0,0); unknown method → empty result. `didOpen`/`didChange` recorded (diagnostics re-pushed on didOpen only).
- Unit tests (hub pure logic): id counter increments; requests before Ready queue then flush (drive with a fake: spawn `/bin/cat` as "server" — echoes framed bytes back: write a message, cat echoes it, reader parses it → Response event with same id; poisoning on cat exit → Died).
- Integration tests (real mock): `tests/lsp_flow.rs` — spawn plat binary w/ PLAT_LSP_MOCK=1 as server via hub (`cfg override "rust" -> command for current plat bin + env`? hub spawns with env var: `ensure` takes cfg argv; test passes `[env!("CARGO_BIN_EXE_plat")]` and sets PLAT_LSP_MOCK via hub? **pin**: `Server` spawns with env var `PLAT_LSP_MOCK=1` set ONLY when argv[0] == current_exe? NO — cleanest: test config `[lsp] rust = "<path-to-plat-bin>"` and the hub sets the env when spawning? pin the real pin: hub never invents env; instead the mock binary mode is selected by argv[0] name — `ensure` argv may be `["sh","-c","PLAT_LSP_MOCK=1 <bin>"]`. Tests use that form. Document in report.

**Steps:** TDD: integration test "ensure(mock) → Ready → didOpen notify → Diagnostics event arrives with 2 diags" (drain loop w/ 200ms timeout); then kill mid-request test (request completion, drop child, expect Died, hub still usable, **and the main-thread call `request()` returned instantly** — assert elapsed < 50ms). RED → implement → full suite → commit `lsp: hub with per-server reader threads + PLAT_LSP_MOCK dev server`.

### Task 4: attach, document sync, UTF-16 math

**Files:** `src/document.rs` (field `pub edit_gen: u64`, bump in all four choke points — the SAME four that push PendingEdits), `src/lsp/mod.rs` (attach + sync + position utils), `src/app.rs` (`lsp_attach(doc_idx)`, sync pass fn), run loop + flow press call sites.

**Interfaces:**
- `pub fn char_col_to_utf16(line_text: &str, col_chars: usize) -> u32` and inverse `utf16_col_to_char(line_text: &str, col_utf16: usize) -> usize` (in `src/lsp/mod.rs`; line-bounded scans only).
- `App::lsp_attach(&mut self, idx: usize)` — when doc has path + `language_for` + `cfg.command_for` present + root found: `hub.ensure` + queue `didOpen` (TextDocumentItem, full text, uri = `file://{canonical path}`, minimal %20 encoding, ponytail comment). Called from constructor + `open_path` + reload reset path.
- `App::lsp_sync_pass(&mut self, now)` — for each attached doc where `edit_gen` changed since last sync AND ≥250 ms since the newest edit bump: send `didChange` full text, record gen+time. `App::lsp_sync_now(&mut self, idx)` forces immediately (used pre-request).
- didChange/didOpen uri and doc registration per path (hub remembers opened uris so re-attach after reload resends).
- run loop: `app.lsp_sync_pass(now)` each poll; flow press() same.

**Steps:** failing tests: utf16 conversions table (ASCII, `é`, emoji w/ surrogate pair, mixed — badge-relevant: line "é→💯x", cols both ways round-trip); edit_gen bumps exactly once per edit/undo/redo/replace_all splice (extend existing pending_ts tests' docs); integration (mock): open attached .rs file, type, wait ≥250ms (test uses `std::thread::sleep(300)` — acceptable in one ignored=false integration test; keep sleep out of unit tests), assert mock re-pushed diagnostics twice (open+change) via events; forced `lsp_sync_now` pre-request has no sleep. RED → implement → suite → commit `lsp: lazy attach, debounced full-text sync, utf16 position math`.

### Task 5: diagnostics UX — pump, gutter badges, status count, `[`/`]`

**Files:** `src/app.rs` (`lsp_pump()`, Commands PrevDiagnostic/NextDiagnostic handlers, status text), `src/render.rs` (gutter badge column + status segment), `src/keymap.rs` (Action `[`/`]` + names `prev_diagnostic`/`next_diagnostic`), `src/document.rs` (`pub lsp_diags: Vec<lsp_types::Diagnostic>` with ranges converted to char offsets at pump time).

**Interfaces:**
- run loop + flow press: `app.lsp_pump()` after `refresh_branch()` — Diagnostics → convert+store (utf16→char via Task 4 inverse; out-of-range clamps), Response → drop unless pending matches (Task 6/7 consume; store last unmatched in a debug-friendly slot), Died → status note `lsp: {lang} server died` once + respawn-allowed-once flag (second Died for same lang → disable, note `lsp: {lang} disabled`).
- Gutter: `badge_w: u16 = 2` when the doc has ≥1 diagnostic landing on a visible line, else 0; when 2: cell0 = severity char (`E`fg-Red / `W`fg-Yellow / `I`,`H`fg-DarkGrey), cell1 = space, then the usual line-number gutter (so rows byte-identical when no diagnostics).
- Status right side: after percent segment, append ` {n}E {n}W` (each count omitted when 0; nothing when no diags at all).
- Status left messages: cursor on a diagnostic line → `msg` appended after mode/branch/lang segments: first matching diagnostic `line:col: message` (truncated to remaining budget — narrow-width rule: left truncates, right pinned, mode survives).
- Commands: Action `[` → PrevDiagnostic, `]` → NextDiagnostic (wrap; moves cursor to the diag's start line/char; scrolls via existing clamp_viewport).

**Steps:** failing unit tests (render + app with hand-set `lsp_diags`, no server needed): badge column appears only when a visible line has a diag AND gutter shifts numbers right by 2; no diags → bytes identical to today's baselines; status count text; cursor-on-diag message text; `[`/`]` wrap + jump math (two diags at lines 5, 50; from line 0 `]`→5, `]`→50, `]`→5 wrap); keymap lookups. RED → implement → suite (existing gutter baselines MUST pass untouched — Review Focus 5) → commit `lsp: diagnostics gutter badges, status counts, [ ] navigation`.

### Task 6: completion — trigger, menu, insert

**Files:** `src/keymap.rs` (`Command::Completion`; global ctrl arms `Char(' ')`+CTRL and `Char('@')`+CTRL; name `completion`), `src/app.rs` (`completion: Option<CompMenu>` state, pending id + gen capture, key routing when open, insert fn), `src/render.rs` (menu paint), `src/lsp/snippet.rs` (`pub fn strip_snippet(&str) -> String`).

**Interfaces:**
- Trigger: Edit-mode `Tab` currently → `Indent`: keep keymap as-is; in `App` Indent handler: if cursor is on or right after identifier char (`[A-Za-z0-9_]`) AND that lang has an LSP server (config command_for present — do NOT spawn on Tab if no server configured) → request completion instead of indenting. Else indent (unchanged). Ctrl-Space/Ctrl-@ → Command::Completion directly (same server-config gate; do spawn).
- Request: `lsp_sync_now` then `textDocument/completion` params `{textDocument.uri, position: utf16}`; pending = `(id, doc_idx, edit_gen)`.
- `CompMenu { items: Vec<CompItem>, sel: usize, doc_idx, gen }` where `CompItem { label, kind: u8, text_edit: Option<(char_range, String)>, insert: String }` (snippet-stripped at parse time; `kind` = lsp CompletionItemKind → single letter `f` function, `v` variable, `m` module... map table in code).
- Menu open routing (before keymap): `Tab`→sel+1 (wrap), `BackTab`→sel-1, `Enter`→accept, `Esc`→close, any OTHER key → close menu first, then route normally (spec). Menu auto-closes when `doc.edit_gen != menu.gen` at pump.
- Paint: rows at `(cursor display row + 1 + i, cursor display col)`, max 10, one line each `{kind} {label}`, selected row bg=Blue (reuse selection style constant), text default fg. Overlaid after syntax paint, BEFORE selection/search paint blocks (same layering rule: syntax < completion < selection/search? pin: completion menu paints over syntax spans AND selection bg (it is a transient overlay), search underline paints after — menu cells are new set_char writes, they overwrite chars; search on menu cells acceptable/ignored. Record decision in report.)
- Insert (Enter): if `text_edit` range present → doc.edit(range chars, text); else word-start = scan left identifier run from cursor → doc.edit(word_start..cursor, insert). One edit call → generation bumps → menu closes naturally.
- Unit tests: `strip_snippet("beta_x(${1:arg})$2")` → `beta_x(arg)`; `${1}`, `\}`, `\\$` escapes; menu routing (Tab wrap, Enter applies correct textEdit at right char range on an emoji line — Review Focus 3; stale-gen response dropped by pump — Review Focus 4). Flow test with mock: Ctrl-Space → Tab → Enter inserts `alpha_full`; Tab-at-`x` triggers, Tab-on-blank-line still indents.

**Steps:** TDD per above → suite → commit `lsp: completion menu with snippet-strip, textEdit, generation guard`.

### Task 7: goto-definition + jump-back

**Files:** `src/keymap.rs` (`Command::GotoDef` (global `Char(']')`+CTRL — note crossterm delivers Ctrl-] as Char(']')+CONTROL), Action `g` prefix: new `Pending::GotoPrefix` → `d`→GotoDef, `o`→JumpBack; names `goto_def`/`jump_back`), `src/app.rs` (pending id+doc, `jump_stack: Vec<(Option<PathBuf>, usize /*char*/)>` cap 50, jump apply).

**Interfaces:**
- Request like completion (forced sync); pending `(id, doc_idx)` — NO generation drop (intentional navigation; Review Focus 4 documented test).
- Parse result `serde_json::Value` → `lsp_types::GotoDefinitionResponse` (Location | Location array | LocationLink): single → apply; array → first + status `{n-1} more locations (list in phase 2)`; null/empty → `no definition found`.
- Apply: push current (path, cursor) onto jump_stack; target file == active doc path → set cursor, center view (`scroll_row = line.saturating_sub(view_rows/2)` — pin view_rows from current render calc or a stored field; report); else find open buffer (same canonical path) → activate; else open new buffer (existing open_path behavior) → set cursor+center.
- `JumpBack`: pop stack → switch/activate + restore cursor (no re-push).
- Tests: keymap arms + pending prefix; unit apply tests (same-file cursor+scroll centering; other-file reuses open buffer, else opens); stale edit + goto-def STILL applies (the rule); jump-back twice unwinds; cap 50. Flow (mock): Ctrl-] → cursor lands line 0 (mock location), status clean; `g d` from Action mode same; `g o` returns to original spot.

**Steps:** TDD → suite → commit `lsp: goto-definition jump + jump-back stack`.

### Task 8: perf benches, README, PLAN, mock pty smoke

**Files:** `src/lsp/*` benches (`#[ignore]` in-module), README.md, PLAN.md.

**Steps:**
1. Benches (release, ignored): keystroke dispatch on 100k-line attached buffer vs baseline helper (measure `handle_command(InsertChar)` loop w/ and w/o hub events pending — report both, gate ≤ +10%); 100k-line full text framed write (single `write_message`, mock stdin sink) expect <10ms; `lsp_pump` with 1000 queued diagnostic events ≤1ms. Run `cargo test --release -- --ignored lsp`, record REAL numbers (honest reporting like syntax feature).
2. README: "Language servers" feature section (config table incl. defaults, keys table addition, mock note for tests, phase-2 list) + "Acceptance (LSP core)": counts, Review Focus behaviors → named tests, perf numbers, binary size.
3. PLAN.md #5: mark phase-1 shipped with the phase-2 remainder named explicitly.
4. pty smoke `/tmp/opencode/lsp_smoke.py` (reuse tabs_smoke skeleton): temp dir with a `.rs` file + temp config `[lsp] rust = "sh -c 'PLAT_LSP_MOCK=1 <platbin>'"`; assert status shows diagnostics count (`1E 1W`), cursor-on-line shows `mock error`, Ctrl-] moves cursor (screen row check), Ctrl-Q exit 0, no panicked.
5. Final suites; commit `docs+benches: LSP core acceptance`.

---

## Self-review notes

- Spec coverage: transport T1; config T2; lifecycle/threads/mock T3; sync+utf16 T4; diagnostics UX T5; completion T6; goto-def T7; gates/docs/smoke T8. Error handling spread T3 (died/respawn/poison) + T5 (notes) — spec's "never fail editing" tested in T3 kill-mid-flight + T5 zero-footprint.
- Review Focus → tests: 1 → T3 (instant return, Died); 2 → T1 (byte-at-a-time, poisoning); 3 → T4 (utf16 table) + T6 (textEdit on emoji line); 4 → T6 (generation drop) + T7 (intentional apply); 5 → T5 (byte-identical gutter baselines) + T2 (false→None).
- Pinned cross-task names: `LspEvent::{Ready,Response,Diagnostics,Died}`, `LspHub::{ensure,request,notify,take_events,shutdown_all}`, `Document::edit_gen`, `Document::lsp_diags`, `App::{lsp, lsp_attach, lsp_sync_pass, lsp_sync_now, lsp_pump}`, `CompMenu`/`CompItem`, `Command::{Completion,GotoDef,JumpBack,PrevDiagnostic,NextDiagnostic}`, `strip_snippet`, `char_col_to_utf16`/`utf16_col_to_char`, mock env `PLAT_LSP_MOCK` (spawned via `sh -c` argv in tests — hub sets no env).
- Sleep policy: only ONE test sleeps (300 ms debounce integration); everything else event-driven.
