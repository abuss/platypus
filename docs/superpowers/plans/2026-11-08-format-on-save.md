# Format-on-Save Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opt-in `format_on_save`: pressing save (`C-s` / `:w`) saves the file, asks the language server to format it, applies the returned TextEdits as one undo batch, and rewrites the file.

**Architecture:** Reuses the existing async LSP pattern (`pending_completion` / `pending_goto`): save stays synchronous — the file is written first, then a `textDocument/formatting` request fires. A new `format_pump()` inside `App::lsp_pump()` consumes the response, converts utf16 ranges to char offsets, and applies edits through a new `Document::apply_text_edits` (same descending-offset batch pattern as `replace_all_from`). If the server is missing, disabled, or answers `null`/error, the save is simply a normal save. No formatters ship inside plat — the language server does the work.

**Tech Stack:** Rust, ropey, serde_json, lsp-types (already in tree). Mock LSP server at `src/lsp/mock.rs` (`PLAT_LSP_MOCK=1 plat`).

**Spec:** issue #10 (LSP phase 2 slice: format-on-save only; rename/hover/etc. stay open).

## Global Constraints

- Opt-in: `format_on_save` defaults to **false** in `[editor]` config.
- Formatting edits must be **one undo step** (Ctrl-Z reverts all of them at once).
- All existing gates: `cargo test` green (unit + `tests/flow.rs` + `tests/lsp_flow.rs`), zero build warnings, binary < 50 MB, pty smoke stable ×3.
- Save itself must never stall or fail because of formatting: the disk write happens before the format request; the formatted rewrite is a second write.
- `:wq`, close-confirm saves, and SaveAs keep saving synchronously (no format hook) — quitting must never wait on a server.
- Branch `feat/format-on-save` off `main`; commit per task; push + PR at the end.

## Review Focus

- Server that doesn't support formatting → error response arrives as `Response { result: Null }` (see `classify_message`, src/lsp/mod.rs:460-463) → save must complete, no edits, no status error. Test: `format_response_null_is_noop`.
- TextEdit ranges past end-of-line or past EOF (servers clamp differently) → clamp like the diagnostics converter does (src/app.rs:285-293). Test: `format_clamps_out_of_range_cols`.
- Buffer edited between the save and the format response → applying stale offsets would splice garbage → drop the edits silently. Test: `format_stale_when_doc_edited_after_request`.
- CRLF buffer: server `newText` uses `\r\n` or `\n` → rope stays LF, save re-encodes CRLF; normalize `\r\n` → `\n` in newText before applying. Test: `format_normalizes_crlf_newtext`.
- Formatting one edit must not appear as many undo steps → single batch. Test: `format_applies_as_one_undo_step`.

---

### Task 1: `format_on_save` config key

**Files:**
- Modify: `src/config.rs` (EditorCfg ~l.25-45, RawEditor parse ~l.259-277, suggestion list ~l.373, example TOML in tests ~l.458)
- Test: `src/config.rs` (inline `mod tests`)

**Interfaces:**
- Produces: `Config::editor.format_on_save: bool` — read by App in Task 4.

- [ ] **Step 1: Failing tests** in `src/config.rs` tests:
```rust
#[test]
fn format_on_save_defaults_false() { /* Config::defaults() → editor.format_on_save == false */ }

#[test]
fn format_on_save_parses_true() {
    // parse "[editor]\nformat_on_save = true\n" → editor.format_on_save == true
}
```
- [ ] **Step 2:** Run `cargo test config 2>&1 | tail` → FAIL (no field).
- [ ] **Step 3:** Add `pub format_on_save: bool` to `EditorCfg` (+ `false` in its `Default`), `format_on_save: Option<bool>` to `RawEditor` (+ `#[serde(default)]`), apply in the editor-section block, and add `"format_on_save"` to `all_editor_fields` (typo-suggestion list at ~l.373).
- [ ] **Step 4:** `cargo test config 2>&1 | tail` → PASS.
- [ ] **Step 5:** Commit `feat(config): format_on_save option`.

### Task 2: `Document::apply_text_edits` (batched, descending)

**Files:**
- Modify: `src/document.rs` (add method next to `replace_all_from`, ~l.418-445)
- Test: `src/document.rs` inline tests

**Interfaces:**
- Produces: `pub fn apply_text_edits(&mut self, edits: Vec<(usize, usize, String)>) -> usize` — `(start_char, end_char, new_text)`, pre-converted char offsets; sorts descending internally; single `history.push_batch`; pushes `pending_ts` when syntax is active (mirror `replace_all_from`'s pending_ts block exactly); sets `dirty`, bumps `edit_gen`; returns count. Consumed by Task 4.

- [ ] **Step 1: Failing tests:**
```rust
#[test]
fn apply_text_edits_is_one_undo_step() {
    // rope "a b a b", edits [(0,1,"X"),(4,5,"Y")] → rope "X b X b";
    // history.undo once → back to "a b a b"; second undo → None; dirty==true.
}
#[test]
fn apply_text_edits_insert_and_multiline_replacement() {
    // [(0,0,"// fmt\n".into())] inserts at line start; empty batch → 0, no history entry, no edit_gen bump.
}
```
- [ ] **Step 2:** Run `cargo test apply_text_edits 2>&1 | tail` → FAIL.
- [ ] **Step 3:** Implement: sort `edits` by `start` descending (stable), loop: capture `slice`, push `pending_ts` when `self.syntax.is_some()` (same pattern as `replace_all_from`), `rope.remove(s..e); rope.insert(s, &new)`, push `Change`. Then `if n>0 { push_batch; dirty=true; edit_gen+=1 }`. Comment the descending-order/pending_ts invariant like l.429-431 does.
- [ ] **Step 4:** `cargo test document 2>&1 | tail` → PASS.
- [ ] **Step 5:** Commit `feat(document): batched apply_text_edits`.

### Task 3: LSP hub — formatting capability + mock server support

**Files:**
- Modify: `src/lsp/mod.rs` (initialize params, ~l.174-183)
- Modify: `src/lsp/mock.rs` (initialize capabilities l.26-33; new method arm before `shutdown`)
- Test: `tests/lsp_flow.rs` (append one test)

**Interfaces:**
- Produces: mock answers `textDocument/formatting` with `[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"// fmt\n"}]`. Consumed by Task 4's smoke and the flow test.

- [ ] **Step 1: Failing flow test** `tests/lsp_flow.rs`:
```rust
#[test]
fn test_formatting_request() {
    // ensure(mock) → wait Ready → didOpen → request "textDocument/formatting"
    // with params {"textDocument":{"uri":...},"options":{"tabSize":2,"insertSpaces":true}}
    // wait_event Response{result} → result is an array of 1 with newText == "// fmt\n"
}
```
- [ ] **Step 2:** Run `cargo test --test lsp_flow test_formatting_request 2>&1 | tail` → FAIL (mock's unknown-method arm returns `{}`).
- [ ] **Step 3:** In mock.rs: add `"documentFormattingProvider": true` to the initialize result's capabilities and the `textDocument/formatting` arm above. In `src/lsp/mod.rs` initialize params, replace `"capabilities": {}` with:
```json
"capabilities": { "textDocument": { "formatting": {} } }
```
(one line; keep the whole-client-caps expansion out of scope).
- [ ] **Step 4:** `cargo test --test lsp_flow 2>&1 | tail` → PASS (all flow tests, mock still answers other methods as before).
- [ ] **Step 5:** Commit `feat(lsp): declare + serve documentFormatting`.

### Task 4: App — save hook, `pending_format`, `format_pump`

**Files:**
- Modify: `src/app.rs` (struct field ~l.88 and ctor ~l.231; `Save` arm l.1382; colon `:w` arm ~l.1963; `lsp_pump` ~l.303-358; new methods near `goto_pump`)
- Test: `src/app.rs` inline tests (follow the `pending_goto` test style, l.4538+)

**Interfaces:**
- Consumes: Task 1 `config.editor.format_on_save`; Task 2 `Document::apply_text_edits(Vec<(usize,usize,String)>) -> usize`; Task 3 mock.
- Produces: `pub pending_format: Option<(u64, PathBuf, u64)>` (id, doc path, `edit_gen` at request time); `fn maybe_format_after_save(&mut self, idx: usize)`; `fn format_pump(&mut self)` called from `lsp_pump` next to `completion_pump`/`goto_pump`.

- [ ] **Step 1: Failing tests** (drive pump directly, no real server — `pending_goto`-test style):
```rust
#[test] fn format_disabled_no_request()          // default config, Command::Save → pending_format None
#[test] fn format_on_save_requests_after_save()  // format_on_save=true, [lsp] rust=["sh","-c","sleep 30"];
                                                 // Command::Save on a .rs file → pending_format Some, file on disk == unformatted text
#[test] fn format_response_applies_and_resaves() // tmp file "a\nb\n"; set pending_format; push last_responses (id, edits replacing line 0 with "Z");
                                                 // lsp_pump() → buffer "Z\nb\n", DISK "Z\nb\n", dirty false, status "formatted"
#[test] fn format_applies_as_one_undo_step()     // same, then Command::Undo → "a\nb\n" in buffer
#[test] fn format_response_null_is_noop()        // result Null → buffer/disk untouched, pending cleared
#[test] fn format_stale_when_doc_edited_after_request() // edit_gen advanced after request → edits dropped, buffer/disk keep edited text
#[test] fn format_clamps_out_of_range_cols()     // end character 999 on line "ab" → replaces whole line, no panic
#[test] fn format_normalizes_crlf_newtext()      // newText "x\r\ny" → buffer has "x\ny"
#[test] fn responses_for_format_survive_retain() // pending_format id is kept by the last_responses.retain in lsp_pump
```
- [ ] **Step 2:** Run `cargo test format 2>&1 | tail` → FAIL (field/methods missing).
- [ ] **Step 3:** Implement:
  - `pending_format` field + ctor init.
  - `maybe_format_after_save(idx)`: gate = `config.editor.format_on_save` && doc has path && `syntax::language_for` Some && `lsp::command_for` Some && `!lsp_disabled.contains(lang)` (mirror `trigger_completion`'s gates, l.614-631). Then `lsp_attach(idx)`, `lsp_sync_now(idx)`, mirror l.643-651: `request(lang, &root, "textDocument/formatting", json!({"textDocument":{"uri":uri},"options":{"tabSize":self.config.editor.indent_width as u64,"insertSpaces":true}}))` → store `pending_format = Some((id, path, edit_gen))`. Call it after `Ok(())` in the `Save` arm (l.1382-1385) and after Ok in the colon `:w` arm (~l.1963) — both keep the "saved" status.
  - `format_pump()`: mirror `goto_pump` (l.696-703): take pending, find matching id in `last_responses`, parse result as `Vec<Value>` (null/non-array → clear + return). For each edit: clamp line to `rope.len_lines()`, convert start/end utf16 cols with `lsp::utf16_col_to_char(line_content, …).min(line_len)` exactly like the diagnostics converter (l.285-293), char offsets via `rope.line_to_char(line) + col`; skip edits landing past `rope.len_chars()`; `newText.replace("\r\n", "\n")`. Find the doc by path (same way the Diagnostics arm does); if `docs[i].edit_gen != req_gen` drop. Apply with `docs[i].apply_text_edits(converted)`; if >0: `let _ = self.docs[i].save(); self.status = Some(("formatted".into(), false));`.
  - Call `self.format_pump();` right after `self.goto_pump();` (l.347) and extend the retain at l.351-353 with `|| Some(*id) == pend_fmt`.
- [ ] **Step 4:** `cargo test 2>&1 | tail` → all green (unit + flow + lsp_flow).
- [ ] **Step 5:** Commit `feat(lsp): format on save (opt-in)`.

### Task 5: pty smoke + docs

**Files:**
- Create: `/tmp/opencode/format_save_smoke.py`
- Modify: `README.md` (config table + a Format-on-save section), `PLAN.md` (ledger line), config example test in `src/config.rs` if it lists editor fields

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1:** Smoke (model on `/tmp/opencode/preview_smoke.py`): tmpdir with `config.toml` (`[editor] format_on_save = true`, `[lsp] rust = ["sh","-c","PLAT_LSP_MOCK=1 <bin>"]`) and `x.rs` containing `fn main() {}`. Pty-launch `plat --config config.toml x.rs` (Esc and any colon keystrokes sent as SEPARATE pty writes — ESC+`:` decodes as Alt-`:`, known PTY fact). Checks: (1) startup renders, (2) send `C-s`, poll until `x.rs` on disk starts with `// fmt` (timeout 5 s), (3) status bar shows `formatted`, (4) file on disk is not double-formatted after a second `C-s` (exactly one `// fmt` line — server re-format returns insert-at-0 again, so instead assert disk content is stable across two saves 1 s apart if (2) flake; keep it simple: one save, one check). Run `python3 /tmp/opencode/format_save_smoke.py` ×3 → all pass.
- [ ] **Step 2:** README: add `format_on_save` to the config section (default off; needs a language server; one undo reverts formatting) + acceptance bullets. PLAN.md: mark #10 format-on-save slice shipped.
- [ ] **Step 3:** Full gate: `cargo build --release 2>&1 | grep -c warning` → 0; `cargo test 2>&1 | tail` green; `ls -la target/release/plat` < 50 MB.
- [ ] **Step 4:** Commit `docs: format-on-save`; push branch, `gh pr create` against main.

## Install (after merge, per repo convention)

`cp target/release/plat ~/.local/bin/plat.new && mv -f ~/.local/bin/plat.new ~/.local/bin/plat` — user must fully restart plat.
