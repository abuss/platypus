# Workspace Symbols Outline — Design Document

**Date**: 2026-11-14  
**Issue**: #61  
**Scope**: Two LSP-backed symbol navigation modes in sidebar  
**Related**: Issue #10 (LSP Phase 2)

## Overview

Add two sidebar modes for navigating code symbols via LSP:

- **File Symbols** (`Ctrl-O`): All symbols in the current file
- **Workspace Symbols** (`Shift-Ctrl-O`): Searchable symbols across entire workspace

Both modes allow users to jump to symbol definitions while keeping the sidebar open for continuous browsing. A tree-sitter fallback ensures symbols appear even without LSP.

## Requirements & Decisions

### User-Facing Behavior

| Aspect | Decision |
|--------|----------|
| Jump behavior | Jump to symbol, **keep sidebar open** (continuous browsing) |
| Filter strategy | Server-side LSP query filtering (per keystroke) |
| Result limit | First N results (default 100) with truncation indicator |
| Symbol source | Tree-sitter primary, LSP fallback if available |
| Keybindings | `Ctrl-O` (file), `Shift-Ctrl-O` (workspace), + command palette |

### Why These Choices

- **Keep sidebar open**: Users often jump through multiple symbols. Closing/reopening is friction.
- **Server filtering**: Workspaces can have thousands of symbols; filtering on server is cheaper and avoids network overhead of transmitting entire list.
- **First N results**: Full pagination UI is future scope; truncation indicator signals more exist.
- **Tree-sitter + LSP**: Tree-sitter works immediately (no server startup), LSP provides richer data (type info, containers) when available.
- **Command palette**: Discoverable for new users who don't know keybindings.

## Architecture

### Data Model

#### Symbol State Structs (src/sidebar.rs)

```rust
/// Parsed LSP symbol with location info for jumping
#[derive(Debug, Clone)]
pub struct LspSymbol {
    pub name: String,
    pub kind: SymbolKind,           // LSP: Function, Class, Variable, etc.
    pub range: (u32, u32, u32, u32), // (start_line, start_col, end_line, end_col)
    pub detail: Option<String>,     // Type signature / context
    pub container: Option<String>,  // For workspace symbols: parent scope
}

/// File symbols from current buffer (textDocument/documentSymbol)
#[derive(Debug, Clone)]
pub struct FileSymbolState {
    pub items: Vec<TreeItem>,       // Display rows (symbol name + kind icon)
    pub symbols: Vec<LspSymbol>,    // Corresponding LSP data for jumping
    pub cursor: usize,               // Selected row index
    pub truncated: bool,             // More symbols exist but not shown
    pub cached_doc_uri: String,     // Track which file these symbols are from
}

/// Workspace symbols with query filtering (workspace/symbol)
#[derive(Debug, Clone)]
pub struct WorkspaceSymbolState {
    pub items: Vec<TreeItem>,
    pub symbols: Vec<LspSymbol>,
    pub cursor: usize,
    pub truncated: bool,
    pub query: String,              // Current search query
    pub pending: bool,              // Request in-flight
    pub last_query_time: std::time::Instant, // For debouncing
}

/// Updated enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    TreeBrowser,
    FuzzyPicker,
    SearchResults,
    FileSymbols,
    WorkspaceSymbols,
}
```

### LSP Integration (src/lsp/mod.rs)

#### File Symbols Request

**Method**: `textDocument/documentSymbol`

**When**: User presses `Ctrl-O` in active editor

**Request**:
```json
{
  "method": "textDocument/documentSymbol",
  "params": {
    "textDocument": { "uri": "file:///path/to/file.rs" }
  }
}
```

**Response** (LSP DocumentSymbol):
```json
[
  {
    "name": "handle_command",
    "kind": 12,
    "range": { "start": {"line": 2903, "character": 0}, "end": {"line": 3000, "character": 0} },
    "selectionRange": {"start": {"line": 2903, "character": 4}, "end": {"line": 2903, "character": 18}},
    "detail": "fn(&mut self, Command) -> bool"
  }
]
```

**Parsing**: Extract name, kind, range. Store in `FileSymbolState.symbols`.

#### Workspace Symbols Request

**Method**: `workspace/symbol`

**When**: User presses `Shift-Ctrl-O` or types in workspace symbols mode

**Request** (per keystroke, debounced 200ms):
```json
{
  "method": "workspace/symbol",
  "params": {
    "query": "handle_command"
  }
}
```

**Response** (LSP SymbolInformation):
```json
[
  {
    "name": "handle_command",
    "kind": 12,
    "location": {
      "uri": "file:///path/to/app.rs",
      "range": {"start": {"line": 2903, "character": 0}, "end": {"line": 2903, "character": 18}}
    },
    "containerName": "impl App"
  }
]
```

**Parsing**: Extract name, kind, location (uri + range). Store in `WorkspaceSymbolState.symbols`.

**Debouncing**: Only send request if 200ms has elapsed since last keystroke and query changed.

#### Mock Handlers (src/lsp/mock.rs)

Both methods have mock implementations returning test symbol data:

```rust
Some("textDocument/documentSymbol") => {
    // Return test symbols for current buffer
    serde_json::json!([
        {"name": "main", "kind": 12, ...},
        {"name": "helper", "kind": 12, ...},
    ])
}

Some("workspace/symbol") => {
    // Filter mock symbols by query param
    // Return first 100 results
}
```

### Commands (src/keymap.rs)

#### New Command Enum Variants

```rust
pub enum Command {
    // ... existing ...
    FileSymbols,
    WorkspaceSymbols,
}
```

#### Keybindings (Action Mode)

```rust
impl Keymap {
    // In lookup() function, Action mode handler:
    KeyCode::Char('o') if plain => Some(Command::FileSymbols),
    KeyCode::Char('O') if key.modifiers == KeyModifiers::SHIFT => Some(Command::WorkspaceSymbols),
}
```

#### Command Palette Entries

```rust
pub const PALETTE_ITEMS: &[PaletteItem] = &[
    // ... existing ...
    PaletteItem {
        cmd: Command::FileSymbols,
        name: "file_symbols",
        desc: "show symbols in current file (LSP)",
        default_hint: Some("C-o"),
    },
    PaletteItem {
        cmd: Command::WorkspaceSymbols,
        name: "workspace_symbols",
        desc: "search symbols in workspace (LSP)",
        default_hint: Some("C-S-o"),
    },
];
```

### Application Logic (src/app.rs)

#### Command Handlers

```rust
impl App {
    pub fn handle_command(&mut self, cmd: Command) {
        match cmd {
            // ... existing ...
            Command::FileSymbols => {
                self.request_file_symbols();
                self.open_sidebar(SidebarMode::FileSymbols);
            }
            Command::WorkspaceSymbols => {
                self.open_sidebar(SidebarMode::WorkspaceSymbols);
                // Sidebar starts with empty query; typing triggers requests
            }
        }
    }

    /// Request file symbols from LSP for active buffer
    fn request_file_symbols(&mut self) {
        let Some(active_doc) = self.active_doc_mut() else {
            self.status = Some(("No active buffer".into(), false));
            return;
        };

        let uri = &active_doc.uri;
        let doc_uri = format!("file://{}", active_doc.path.display());

        // Send textDocument/documentSymbol request
        if let Some(req_id) = self.lsp.request("rust", &self.tree_root, "textDocument/documentSymbol", json!({
            "textDocument": { "uri": doc_uri }
        })) {
            self.pending_lsp_requests.insert(req_id, "file_symbols");
        } else {
            // Fallback: use tree-sitter symbols
            self.load_file_symbols_from_tree_sitter();
        }
    }

    /// Handle LSP response for file/workspace symbols
    fn on_symbol_response(&mut self, req_id: u64, result: Value) {
        let request_type = self.pending_lsp_requests.remove(&req_id).unwrap_or_default();

        match request_type {
            "file_symbols" => self.parse_file_symbols(result),
            "workspace_symbols" => self.parse_workspace_symbols(result),
            _ => {}
        }
    }

    /// Parse textDocument/documentSymbol response
    fn parse_file_symbols(&mut self, symbols: Value) {
        let mut state = FileSymbolState {
            items: Vec::new(),
            symbols: Vec::new(),
            cursor: 0,
            truncated: false,
            cached_doc_uri: self.active_doc().map(|d| d.uri.clone()).unwrap_or_default(),
        };

        // Parse JSON array into LspSymbol structs
        if let Ok(syms) = serde_json::from_value::<Vec<LspSymbol>>(symbols) {
            for sym in syms {
                // Create TreeItem display row
                let icon = symbol_kind_icon(&sym.kind);
                let row_text = format!("{} {}", icon, sym.name);
                state.items.push(TreeItem { text: row_text, ... });
                state.symbols.push(sym);
            }
        }

        if let Some(sidebar) = &mut self.sidebar {
            sidebar.file_symbol_state = Some(state);
        }
    }

    /// Parse workspace/symbol response
    fn parse_workspace_symbols(&mut self, symbols: Value) {
        // Similar to file symbols, but update workspace_symbol_state
        // Check for truncation (if len >= MAX_RESULTS, set truncated = true)
    }

    /// On keystroke in workspace symbols mode, send query to LSP
    fn on_workspace_symbol_query(&mut self, query: String) {
        if let Some(sidebar) = &mut self.sidebar {
            if sidebar.mode != SidebarMode::WorkspaceSymbols {
                return;
            }

            if let Some(state) = &mut sidebar.workspace_symbol_state {
                state.query = query.clone();
                
                // Debounce: only send if 200ms since last request
                if state.last_query_time.elapsed() > Duration::from_millis(200) {
                    if let Some(req_id) = self.lsp.request("rust", &self.tree_root, "workspace/symbol", json!({
                        "query": query
                    })) {
                        self.pending_lsp_requests.insert(req_id, "workspace_symbols");
                        state.pending = true;
                        state.last_query_time = std::time::Instant::now();
                    }
                }
            }
        }
    }

    /// Jump to symbol location
    fn jump_to_symbol(&mut self, symbol: &LspSymbol) {
        if let Some(sidebar) = &self.sidebar {
            let mode = sidebar.mode;
            
            let uri_str = match mode {
                SidebarMode::FileSymbols => {
                    if let Some(state) = &sidebar.file_symbol_state {
                        state.cached_doc_uri.clone()
                    } else {
                        return;
                    }
                }
                SidebarMode::WorkspaceSymbols => {
                    if let Some(state) = &sidebar.workspace_symbol_state {
                        // Extract URI from symbol location
                        // (requires extending LspSymbol to store source URI)
                        return;
                    } else {
                        return;
                    }
                }
                _ => return,
            };

            // Parse file:// URI to path
            let path = PathBuf::from(uri_str.replace("file://", ""));

            // Open file if not already open
            if !self.docs.iter().any(|d| d.path == path) {
                self.open_file(path);
            }

            // Set cursor to symbol range start
            let (line, col, _, _) = symbol.range;
            self.set_cursor(line as usize, col as usize);

            // **Keep sidebar open** (don't close it)
        }
    }

    /// Fallback: extract symbols using tree-sitter
    fn load_file_symbols_from_tree_sitter(&mut self) {
        if let Some(active_doc) = self.active_doc() {
            if let Some(syntax_state) = &active_doc.syntax {
                let symbols = syntax_state.symbol_chain(active_doc.cursor);
                // Convert to TreeItems and populate FileSymbolState
            }
        }
    }
}
```

### Rendering (src/render.rs)

#### Symbol Row Format

```
<icon> <symbol-name>              @ <location>
```

Example rows:
```
ƒ handle_command                  @ app.rs:2903
Ⓣ App                             @ app.rs:100
♦ Mode                            @ keymap.rs:45
```

#### Kind Icons (Unicode)

```rust
fn symbol_kind_icon(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "ƒ",
        SymbolKind::Struct | SymbolKind::Class => "Ⓣ",
        SymbolKind::Enum => "♦",
        SymbolKind::Variable => "Ϸ",
        SymbolKind::Method => "ƒ",
        SymbolKind::Property => "Ⓟ",
        SymbolKind::Interface => "◇",
        SymbolKind::Module => "⊕",
        SymbolKind::Constant => "≡",
        _ => "•",
    }
}
```

#### Sidebar Rendering

In `render_sidebar()`, add cases for `FileSymbols` and `WorkspaceSymbols`:

```rust
fn render_sidebar(&self, frame: &mut Frame) {
    match sidebar.mode {
        SidebarMode::FileSymbols => {
            // Render symbol list rows
            for (idx, (item, symbol)) in items.iter().zip(symbols).enumerate() {
                let selected = idx == cursor;
                let row_text = format!("{} {}   @ {}:{}", 
                    symbol_kind_icon(&symbol.kind),
                    symbol.name,
                    "buffer.rs",  // File name
                    symbol.range.0 + 1  // 1-indexed line
                );
                render_row(frame, row_text, selected);
            }
        }
        SidebarMode::WorkspaceSymbols => {
            // Render search input at top
            render_input_line(frame, "Search: ", &query);
            
            // Render symbol list below
            for (idx, symbol) in symbols.iter().enumerate() {
                let selected = idx == cursor;
                let row_text = format!("{} {}   @ {}:{}", 
                    symbol_kind_icon(&symbol.kind),
                    symbol.name,
                    extract_filename(&symbol.location_uri),
                    symbol.range.0 + 1
                );
                render_row(frame, row_text, selected);
            }
            
            // Truncation indicator
            if truncated {
                render_row(frame, "... and more symbols", false);
            }
            
            // Pending indicator
            if pending {
                render_row(frame, "[searching...]", false);
            }
        }
        // ... other modes ...
    }
}
```

## Interaction Flow

### File Symbols Flow

1. **Trigger**: User presses `Ctrl-O` in action mode
2. **Request**: App sends `textDocument/documentSymbol` LSP request for active buffer
3. **Response**: LSP server returns list of symbols
4. **Parse**: Convert to `FileSymbolState` with TreeItems + LspSymbols
5. **Render**: Sidebar shows symbol list, cursor on first symbol
6. **Browse**: Arrow keys move cursor, Esc closes sidebar
7. **Jump**: Press Enter on symbol
   - Extract symbol location from range
   - Navigate to file + line
   - Keep sidebar open
8. **Refine**: User can close sidebar with Esc or continue jumping

### Workspace Symbols Flow

1. **Trigger**: User presses `Shift-Ctrl-O` in action mode
2. **Init**: Sidebar opens with empty `WorkspaceSymbolState`, cursor ready for input
3. **Type**: User types search query (e.g., "handle_")
4. **Debounce**: After 200ms without keystroke, request sent
5. **Request**: App sends `workspace/symbol` LSP request with query
6. **Response**: LSP server returns matching symbols (up to 100)
7. **Parse**: Convert to `WorkspaceSymbolState`
8. **Render**: 
   - Show search input at top with query
   - List matching symbols below
   - If results == 100, show "... and more symbols"
9. **Refine**: User types more to filter results (back to step 3)
10. **Jump**: Enter on symbol, sidebar stays open
11. **Exit**: Esc closes sidebar

## Testing Strategy

### Unit Tests

#### src/sidebar.rs
- `test_parse_file_symbols_response`: Parse LSP JSON into `FileSymbolState`
- `test_parse_workspace_symbols_response`: Parse LSP JSON into `WorkspaceSymbolState`
- `test_symbol_truncation`: Verify truncation flag set when results >= limit
- `test_symbol_kind_icon`: Each kind maps to correct Unicode icon

#### src/lsp/mod.rs
- `test_file_symbols_request_format`: Request has correct `textDocument` URI
- `test_workspace_symbols_request_format`: Request includes query param
- `test_workspace_symbols_debounce`: Multiple rapid keystrokes produce single request

#### src/lsp/mock.rs
- `test_mock_file_symbols_response`: Mock returns valid DocumentSymbol array
- `test_mock_workspace_symbols_filtered`: Mock filters by query, limits results

#### src/render.rs
- `test_render_symbol_rows`: Row format includes icon, name, location
- `test_render_truncation_indicator`: Indicator appears when truncated = true
- `test_render_pending_indicator`: "[searching...]" appears when pending = true

### Flow Tests (src/app.rs)

- `test_ctrl_o_opens_file_symbols_sidebar`: Keybinding triggers correct mode
- `test_shift_ctrl_o_opens_workspace_symbols_sidebar`: Keybinding works
- `test_workspace_symbol_query_sends_lsp_request`: Typing → LSP request after debounce
- `test_enter_on_symbol_jumps_to_location`: Cursor moves to symbol, sidebar stays open
- `test_file_symbols_tree_sitter_fallback`: Symbols appear even without LSP response
- `test_symbol_in_different_file`: Workspace symbol jump opens correct file

### Manual Integration Tests

- Verify LSP connection stays alive through multiple symbol requests
- Test symbol accuracy for Rust, Python, and Go
- Confirm workspace symbol count limits and truncation messages
- Check sidebar update responsiveness during typing

## Edge Cases & Mitigations

| Case | Handling |
|------|----------|
| LSP server crashes | Fallback to tree-sitter for file symbols |
| No active buffer | Show status message, don't open sidebar |
| Empty workspace symbols | Show "No symbols found" message |
| Very large workspace | First N results, truncation indicator, query refinement encouraged |
| Rapid query typing | Debounce; only send request every 200ms |
| Symbol jump to deleted file | Open dialog to re-scan workspace or close sidebar |
| Multiple language servers | Request to highest-priority server (per `lsp.rs` config) |

## Known Limitations & Future Work

### Limitations

1. **Flat Symbol List**: Nested symbols (e.g., methods inside `impl` blocks) shown at top level
   - Future: Hierarchical tree rendering
2. **Result Pagination**: Only first N results shown; no "load more" UI
   - Future: Pagination or infinite scroll
3. **Tree-sitter Fallback**: Only for file symbols; workspace symbols require LSP
   - Future: Tree-sitter workspace index
4. **No Symbol Filtering in File Mode**: All symbols always shown (no fuzzy filter)
   - Future: Client-side filter in file symbols mode too

### Future Enhancements

- Breadcrumb symbol path in status line (issue #59)
- Nested symbol tree view
- Symbol type/signature preview
- Keyboard shortcut for most-frequent symbols
- Symbol search history
- Workspace-wide refactor via symbol

## Implementation Plan

See `docs/superpowers/plans/` for detailed implementation phases.

**Phases**:
1. Data structures + LSP mock handlers
2. File symbols (LSP + tree-sitter fallback)
3. Workspace symbols (LSP + debouncing)
4. Rendering + sidebar integration
5. Keybindings + command palette
6. Tests

**Estimated scope**: 3 PRs, ~800–1200 lines of code

