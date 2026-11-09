# Manual Format (`:format` / `format_document`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Format the active buffer on demand — `:format` colon command and `format_document` palette entry — WITHOUT saving: edits land in the buffer as one undo batch, disk untouched, buffer stays dirty.

**Architecture:** Reuses the merged format-on-save plumbing: `App::maybe_format_after_save`'s request core is extracted to `App::request_format(idx, resave)`; `pending_format` grows a `resave: bool`; `format_pump` skips the re-save when `resave=false`. New `Command::FormatDocument` with no default key (CloseBuffer/`markdown_preview` precedent), colon arm `:format` routes to it like `:preview` does.

**Tech Stack:** Rust; existing LSP hub + mock server (`textDocument/formatting` already served).

**Spec:** user-approved design (2026-11-09 conversation): manual trigger, no save, same gates and staleness guard as format-on-save; selection `rangeFormatting` explicitly out of scope.

## Global Constraints

- No default keyboard chord for the command (CloseBuffer/preview precedent); palette + colon only.
- Manual format NEVER writes the disk and leaves `dirty` true.
- Existing gates stand: path + `language_for` + `command_for` + not disabled/spawn-failed; staleness via `edit_gen`.
- All gates: `cargo test` green, zero warnings, binary < 50 MB, pty smoke stable ×3.
- Branch `feat/manual-format` off `main`; commit per task; push + PR at the end.

## Review Focus

- `:format` on a scratch/preview buffer (no path) → status hint, no crash. Test: `format_document_scratch_status_hint`.
- `:format` where the language has a name but the server is disabled (`[lsp] rust = false`) → status hint, no crash, no request. Test: `format_document_no_server_status`.
- User types while a manual format is in flight → stale edits dropped (same guard as save-origin; inherited, no new test).
- After manual format the user expects unsaved state: dirty true, disk unchanged. Test: `manual_format_applies_without_touching_disk`.
- PTY ambiguity: ESC immediately followed by `:` decodes as Alt-`:` → smoke sends Esc and `:format` as separate keystroke writes.

---

### Task 1: Command wiring (keymap + colon + palette)

**Files:**
- Modify: `src/keymap.rs` (enum near `MarkdownPreview` l.289; palette item after l.205; names list ~l.585; mapping ~l.644)
- Modify: `src/app.rs` colon arm: `else if line == "format" { self.handle_command(crate::keymap::Command::FormatDocument); }` next to the `preview` arm (~l.2115)
- Test: `src/keymap.rs` inline + `src/app.rs` inline

**Interfaces:**
- Produces: `Command::FormatDocument`, palette name `format_document`, colon `:format` → dispatches it. Consumed by Task 2 (`format_document()` handler) and the smoke.

- [ ] **Step 1: Failing tests**
```rust
// keymap.rs
#[test] fn format_document_name_maps() {
    assert_eq!(map("format_document"), Some(Command::FormatDocument)); // use the existing name→Command fn
}
#[test] fn format_document_lists_no_default_hint() {
    // palette contains an item with name "format_document" and default_hint None
}
// app.rs
#[test] fn prompt_format_command_routes() {
    // App::new(vec![]) (scratch); CommandPrefix, type "format", PromptSubmit
    // → no new buffer, no panic, status Some((h, true)) (no-server hint, Task 2)
}
```
- [ ] **Step 2:** `cargo test format_document 2>&1 | tail` → FAIL (variant missing). NOTE: `prompt_format_command_routes` may fail with the "unknown command" status — acceptable intermediate; it checks status text only after Task 2. Pin it to assert `!app.docs.is_empty()`-style liveness only? No — assert status message equals "no language server for this file" and accept RED now, GREEN after Task 2.
- [ ] **Step 3:** Add the enum variant, palette item (`desc: "format buffer via language server"`, `default_hint: None`), names-list entry, string mapping, and the colon arm.
- [ ] **Step 4:** `cargo test format_document && cargo test prompt_format` → both PASS (after Task 2 for the status text — if it lags, leave it and Task 2 turns it green; Task 2's contract runs the full suite).
- [ ] **Step 5:** Commit `feat(keymap): format_document command (:format, palette)`.

### Task 2: `format_document()` + `resave` flag in the pump

**Files:**
- Modify: `src/app.rs` (`pending_format` type → `Option<(u64, PathBuf, u64, bool)>`; ctor; extract request core of `maybe_format_after_save` into `fn request_format(&mut self, idx: usize, resave: bool)`; new `pub fn format_document(&mut self)`; `format_pump` honors `resave`; update the 5 test constructions `Some((99, path, eg))` → `Some((99, path, eg, false))`)
- Test: `src/app.rs` inline

**Interfaces:**
- Consumes: Task 1's `Command::FormatDocument`. Produces: `App::format_document()`; colon/palette path applies buffer-only edits; save path behavior byte-for-byte unchanged.

- [ ] **Step 1: Failing tests**
```rust
#[test] fn manual_format_applies_without_touching_disk() {
    // make_fmt_app("m1.rs","a\nb\n"); pending_format=Some((99,path,eg,false));
    // push response replacing line 0 with "Z"; lsp_pump();
    // buffer "Z\nb\n"; DISK "a\nb\n"; dirty true; status "formatted"
}
#[test] fn manual_format_undoes_without_touching_disk() {
    // same, then Command::Undo → buffer "a\nb\n"; disk still "a\nb\n"
}
#[test] fn format_document_scratch_status_hint() {
    // scratch doc; handle_command(Command::FormatDocument);
    // status Some((_, true)); pending_format None
}
#[test] fn format_document_no_server_status() {
    // cfg "[lsp]\nrust = false\n", open .rs, Command::FormatDocument
    // → status contains "no language server", pending_format None
}
```
- [ ] **Step 2:** Run → FAIL (arity/type mismatch + missing behavior).
- [ ] **Step 3:** Implement:
  - `request_format(idx, resave)`: the body of `maybe_format_after_save` minus the `format_on_save` config gate, storing `pending_format = Some((id, path, edit_gen, resave))`.
  - `maybe_format_after_save(idx)`: `if !self.config.editor.format_on_save { return }` then silent gates (return on no-server cases, no status) then `request_format`.
  - `format_document()`: active idx; if no path or no lang → `status = Some(("no language server for this file".into(), true))`; if `command_for` none/disabled/spawn-failed → `status = Some((format!("no language server for {lang}"), true))`; else `request_format(idx, false)` (no status on success).
  - `format_pump`: destructure 4-tuple; after apply `if resave { let _ = save(); baseline_own_write_mtime(doc_idx); }`; status `"formatted"` either way.
- [ ] **Step 4:** `cargo test 2>&1 | tail` → FULL suite green (incl. Task 1's routing test).
- [ ] **Step 5:** Commit `feat(lsp): manual :format / format_document (buffer-only)`.

### Task 3: pty smoke + docs

**Files:**
- Create: `/tmp/opencode/manual_format_smoke.py`
- Modify: `README.md` ("Format on save" section → note manual trigger; palette table entry `format_document`; acceptance bullets), `PLAN.md` (work order #18)

- [ ] **Step 1:** Smoke modeled on `format_save_smoke.py` but config `[lsp] rust = "<wrapper>"` ONLY (no `format_on_save`): (1) plat opens x.rs (`fn main() {}`); (2) send Esc (separate write), then `:format\r` (separate write — ESC+`:` burst decodes as Alt-`:`, known PTY fact); (3) `// fmt` appears in the SCREEN (plain()) within 5 s, (4) file on disk DOES NOT contain `// fmt` after a further 1.5 s, (5) status shows `formatted`, (6) C-q exits 0, no panic. ×3 stable.
- [ ] **Step 2:** README: add `:format` / `format_document` to the format section + acceptance checkboxes (buffer-only, dirty, one undo); PLAN.md work order 18 entry.
- [ ] **Step 3:** Full gate: `cargo build --release 2>&1 | grep -c "^warning"` → 0; `cargo test` green; binary < 50 MB; smoke ×3.
- [ ] **Step 4:** Commit `docs: manual format`; push branch, `gh pr create` against main.

## Install (after merge, per repo convention)

`cp target/release/plat ~/.local/bin/plat.new && mv -f ~/.local/bin/plat.new ~/.local/bin/plat` — user must fully restart plat.
