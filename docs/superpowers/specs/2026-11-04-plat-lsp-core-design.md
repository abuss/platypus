# LSP Core (diagnostics + completion + goto-definition) — Design

Date: 2026-11-04 · Work order: PLAN.md #5 (first phase) · Status: approved conversationally (sections A–D)

## Objective

plat talks to real language servers without ever blocking, freezing, or
distorting typing. Phase one ships exactly three features end-to-end:

1. **publishDiagnostics** — gutter badges, status count, message-on-point, `[`/`]` next/prev.
2. **textDocument/completion** — explicit menu (Ctrl-Space / Tab-at-word), 10 items, Tab/Enter accept.
3. **textDocument/definition** — Ctrl-] jump, `g o` jump back.

Everything else from PLAN.md #5 (rename, format-on-save, snippet tab-stops, multi-location
picker, diagnostics list, hover, workspace symbols, incremental sync, auto-download
servers) is **phase 2** — named here, not built.

## Decisions (binding)

- Client: hand-rolled JSON-RPC/stdio core (~300 lines) + `lsp-types` crate for JSON
  shapes. New deps: `lsp-types`, `serde`, `serde_json` — nothing else (no tokio/async).
- One server per (language, project root), spawned lazily; full-text sync, debounced.
- Reader thread per server; std::sync::mpsc into the main loop. Main loop never waits
  on a server.

## Architecture

### Transport (`src/lsp/transport.rs`)

- Outgoing: `Content-Length: N\r\n\r\n` + JSON (write to server stdin, main-thread only).
- Incoming: reader thread parses frames from server stdout (handles split reads and
  multiple messages per read; headers > 64 KB or non-numeric Content-Length → stream
  error), deserializes with `lsp_types::serde` types, pushes `LspEvent` into
  `mpsc::Sender<LspEvent>`.
- `LspEvent::Response(Id, value)` | `LspEvent::Notification(PublishDiagnostics…) |
  LspEvent::Died`.

### Lifecycle (`src/lsp/mod.rs`, held by `App`)

- Root discovery: walk up from file dir looking for `.git`, `Cargo.toml`,
  `package.json`, `compile_commands.json`; none → file's dir.
- Spawn on first need (any request, or a buffer eligible for diagnostics).
- Handshake: `initialize` (notify `initialized`) with `rootUri`, capabilities:
  `textDocumentSync: Full`, `completion: {}`, `definition: {}`,
  `publishDiagnostics` accepted. Wait for the reply **without blocking input**: the
  buffer enters "attaching" state; requests/didOpen queue until ready.
- On plat exit: `shutdown` request + `exit` notification per server; kill after 300 ms.
- Crash/dead: `Died` event or write error → status note
  (`lsp: rust server died`), one automatic respawn allowed per language per
  session (on next request), then disabled for the session.

### Config (strict `[lsp]` section, additive to schema)

```toml
[lsp]
rust = "rust-analyzer"   # override default command (string = command, spaces split argv)
toml = false             # disable this language
```

- Unknown table keys → normal strict did-you-mean error (existing machinery).
- Defaults (used only if the binary is on PATH):

| lang | default command |
|---|---|
| rust | `rust-analyzer` |
| c | `clangd` |
| toml | `taplo lsp stdio` (probe binary `taplo`) |
| markdown | `marksman` |
| bash | `bash-language-server` (`bash-language-server start`) |
| json | `vscode-json-language-server --stdio` |

- No binary → one status note per language (`no server for rust`), never repeated;
  buffer behaves exactly as plain text.

### Document sync

- `didOpen` when a buffer attaches; `didChange` = **full text** (TextDocumentSyncKind
  Full), sent after 250 ms without an edit to that buffer (idle timer in the run loop,
  same 10 ms tick) and always immediately before completion/goto-def requests.
- Each buffer carries an edit-generation counter (bump in the four mutation paths —
  same choke points as syntax). A response is dropped if the pending request's
  generation no longer matches AND the feature is position-sensitive (completion).
  Goto-def responses are applied regardless (intentional navigation).
- Position math: LSP uses UTF-16 code units. `char↔utf16` conversion is line-bounded
  (scan the line segment only). UTF-16 surrogate pairs ↔ non-BMP chars tested.

### Diagnostics UX

Per-document `Vec<Diagnostic>` (ranges converted to char offsets at receive; dropped
and rebuilt per publish). Rendering:

1. Gutter: +2-char severity badge column (`E` red / `W` yellow / `I` dim / `H` dim),
   shown only when at least one visible line has a diagnostic — otherwise the gutter
   is byte-identical to today.
2. Status right side appends ` 2E 1W` (zero counts omitted; nothing shown when total
   is 0).
3. Cursor on a diagnostic line → that message (first one, truncated to fit) in the
   status line's left part, replacing nothing else — appended like other messages.
4. Action-mode `[`/`]`: cursor to previous/next diagnostic line (wrap; buffer-local).

### Completion UX

- Triggers (Edit mode): `Ctrl-Space` and `Ctrl-@` (same physical key across terminals;
  crossterm reports `Char(' ')`+CONTROL or `Char('@')`+CONTROL), and **Tab when the
  cursor is on or right after an identifier char** (letter/digit/`_`) — Tab elsewhere
  still indents. `completion` stays a remappable command name for the config overlay.
- Request at cursor; response cached as an open menu: max **10 items**
  (`kind` single letter + label, selected first). Menu box drawn inside the editor
  area just below the cursor line (overlaid paint, like search highlights — no
  ratatui widgets).
- Keys while menu open (Edit mode, take precedence over self-insert/motion):
  `Tab` next item (wraps), `Shift-Tab`… arrives as backtab — also cycles;
  `Enter` accept selected, `Esc` dismiss, any text edit/motion dismisses first
  (generation drop), typing after dismissal behaves normally.
- Accept: apply `textEdit` (range + newText) when present, else replace the current
  word prefix with `insertText`. Snippet `insertTextFormat` (2): strip placeholders —
  `${N:TEXT}`→TEXT, `$N`→'', `\}`→`}` etc.; result is plain text. No tab-stops.
- Menu never spans frames without fresh data: one request per trigger; stale
  generations dropped.

### Goto-definition UX

- `Ctrl-]` (global chord, reliable 0x1D byte) at cursor; also Action `g d`
  (new `g` prefix: free today; reserved namespace for phase-2 `g r` rename etc.).
- Response single location (same or other file): move active buffer to file+position
  (reuse already-open buffer for that path; else open into the active buffer slot's
  tab row as a new buffer like open_path). Multiple: take first, status note
  `2 more locations (list in phase 2)`. Empty: status `no definition found`.
- Jump-back: Action `g o` pops a global stack (cap 50 entries: file+cursor) pushed on
  every goto-def jump. Undo/redo do not interact with the stack.

## Error handling (summary)

Nothing on the LSP path can make typing, editing, searching, saving, or quitting
fail. Missing binary, crash, malformed frames, slow replies → status notes (once per
language), degraded plain-text behavior, respawn-then-disable as above. Server
stdout garbage is counted as stream error → dead.

## Performance gates (release, `#[ignore]`, reported honestly)

1. Keystroke dispatch cost unchanged vs pre-LSP baseline on a 100k-line buffer
   (bench existing event path; ≤ +10% tolerance).
2. Debounced full-sync of a 100k-line buffer: single write, measure (expect < 10 ms;
   it is off the key path anyway).
3. Draining 1 000 diagnostics ≤ 1 ms.
4. Startup with servers not configured/installed: no new startup work (spawn is lazy).
5. Binary still < 50 MB (expect ≈ +1 MB).

## Testing

- **`PLAT_LSP_MOCK=1` dev switch** (same precedent as `PLAT_PANIC`): the plat binary
  instead runs as a deterministic stdio LSP server: canned `initialize`, pushes 2
  diagnostics after `didOpen` (one warn one error), completion list of 3 (one
  snippet, one with `textEdit`), definition → one location in the same file
  (line 0). Unit/flow/integration tests spawn the real binary as their server —
  full pipeline, no external servers, no network, no python.
- Unit: framer round-trip/split-read/oversized-header; utf16 char math (BMP + emoji);
  root discovery precedence; `[lsp]` config (override/false/strict unknown key);
  id matching + generation drop; snippet stripping.
- Flow (mock-backed): diagnostics badge + `1E 1W` count + `[`/`]` jumps; Ctrl-Space
  menu → Tab → Enter inserts; Tab-at-word triggers, Tab-on-blank indents; Ctrl-] jumps
  to mock location, `g o` returns; kill mock → status note + editor still works.
- pty smoke: release plat, mock server, real keys, exit 0, no panics.

## Acceptance reporting

README: feature section (config, keys, defaults table), Acceptance (LSP core) with
named tests + gate numbers; PLAN.md #5 marked with phase-2 remainder still open.
