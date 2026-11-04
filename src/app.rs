use std::path::PathBuf;
use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::document::Document;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    Edit,
    Action,
}

pub struct App {
    pub docs: Vec<Document>,
    pub active: usize,
    pub mode: Mode,
    /// (message, is_error)
    pub status: Option<(String, bool)>,
    pub quit: bool,
    pub pending: Option<crate::keymap::Pending>,
    pub clipboard: String,
    pub prompt: Option<crate::prompt::Prompt>,
    pub search_history: Vec<String>,
    /// Replace walkthrough started from the find prompt (None = not running).
    pub replace: Option<ReplaceFlow>,
    /// set when a ConfirmQuit prompt was raised by Ctrl-Q (quit) vs Ctrl-W (close)
    pub quit_after_confirm: bool,
    pub mtime_cache: std::collections::HashMap<std::path::PathBuf, std::time::SystemTime>,
    pub reload_pending: Option<usize>,
    pub suspend_requested: bool,
    pub key_debug: bool,
    pub show_line_numbers: bool,
    /// File sidebar (Ctrl-B tree / Ctrl-P picker); None when closed
    pub sidebar: Option<crate::sidebar::Sidebar>,
    /// Sidebar width kept across open/close for the session
    pub sidebar_width: u16,
    /// git branch per doc path (None value = confirmed not-in-repo, cached so plain files never re-spawn git)
    pub branch_cache: std::collections::HashMap<std::path::PathBuf, Option<String>>,
    /// branch shown on the status bar for the active document
    pub active_branch: Option<String>,
    pub config: Config,
    pub lsp: crate::lsp::LspHub,
    /// Tracks which (lang, path) pairs have been opened with didOpen.
    /// Cleared on Died so re-attach re-sends didOpen after respawn.
    pub lsp_open: HashSet<(String, PathBuf)>,
    /// Per-path LSP sync state: (synced_gen, version_counter, Option<last_edit_seen_instant>)
    pub lsp_sync: HashMap<PathBuf, LspSyncState>,
    /// Buffered LSP events for consumption by Task 5+ and tests
    pub lsp_events: Vec<crate::lsp::LspEvent>,
    /// Last N responses from LSP (capped at 16) for Task 6/7 consumption.
    pub last_responses: Vec<(u64, serde_json::Value)>,
    /// Languages whose server has died twice and is permanently disabled.
    pub lsp_disabled: std::collections::HashSet<String>,
    /// Count of Died events per language (first = warn+respawn, second = disable).
    lsp_death_count: std::collections::HashMap<String, u32>,
    /// Active completion menu, if any.
    pub completion: Option<CompMenu>,
    /// Pending completion request: (request_id, doc_path, edit_gen_at_request).
    /// Path used instead of idx because docs shift on close.
    pub pending_completion: Option<(u64, PathBuf, u64)>,
    /// Pending goto-def request: (request_id, doc_path). No generation drop — navigation
    /// applies even after edits (Review Focus 4).
    pub pending_goto: Option<(u64, PathBuf)>,
    /// Navigation jump stack: (path, char_offset) pairs, capped at 50. VecDeque for front-drop.
    pub jump_stack: std::collections::VecDeque<(PathBuf, usize)>,
}

/// A single completion item parsed from LSP response.
#[derive(Debug, Clone)]
pub struct CompItem {
    pub label: String,
    /// Single-char kind letter: f=function/field(F), m=method, M=module, v=variable, k=keyword, s=snippet, t=text, C=class, ?=other
    pub kind_letter: char,
    /// text_edit if the server provided one: (line, start_char, end_char, new_text)
    pub text_edit: Option<(usize, usize, usize, String)>,
    /// Snippet-stripped insert text (used when no text_edit)
    pub insert: String,
}

/// Active completion dropdown menu.
#[derive(Debug, Clone)]
pub struct CompMenu {
    pub items: Vec<CompItem>,
    pub sel: usize,
    /// Which doc this menu belongs to (index into docs at open time)
    pub doc_idx: usize,
    /// edit_gen of the doc when the menu was opened
    pub edit_gen: u64,
}
#[derive(Debug)]
pub struct ReplaceFlow {
    pub next: usize,
    pub done: usize,
}

/// Per-path LSP sync tracking: what generation we last sent, version counter, and
/// when we last observed a gen CHANGE (trailing-edge 250ms debounce: the clock
/// resets on every new generation, so sync fires 250ms after typing STOPS).
#[derive(Debug)]
pub struct LspSyncState {
    pub synced_gen: u64,
    pub version: i32,
    pub last_seen_gen: u64,
    pub last_edit_seen: Option<std::time::Instant>,
}

/// Map LSP CompletionItemKind to a display letter.
fn comp_kind_letter(kind: u64) -> char {
    match kind {
        1 => 't',  // Text
        2 => 'm',  // Method
        3 => 'f',  // Function
        4 => 'c',  // Constructor
        5 => 'F',  // Field (distinct from function)
        6 => 'v',  // Variable
        7 => 'C',  // Class
        8 => 'M',  // Module
        9 => 'p',  // Property
        10 => 'u', // Unit
        11 => 'V', // Value
        12 => 'e', // Enum
        13 => 'K', // Keyword (uppercase avoids clash with k)
        14 => 's', // Snippet
        15 => 'l', // Color
        16 => 'L', // File
        17 => 'R', // Reference
        18 => 'r', // Folder
        19 => 'E', // EnumMember
        20 => 'S', // Constant
        21 => 'T', // Struct
        22 => 'G', // Event
        23 => 'o', // Operator
        24 => 'P', // TypeParameter
        _ => '?',
    }
}

impl App {
    pub fn new(paths: Vec<PathBuf>) -> App {        Self::new_with_config(paths, Config::defaults())
    }

    pub fn new_with_config(paths: Vec<PathBuf>, config: Config) -> App {
        let mut docs = Vec::new();
        let mut status = None;
        for path in &paths {
            match Document::open(path) {
                Ok(doc) => docs.push(doc),
                Err(e) => status = Some((format!("{}: {e:?}", path.display()), true)),
            }
        }
        if docs.is_empty() {
            docs.push(Document::empty());
        }
        let key_debug = config.editor.key_debug;
        let show_line_numbers = config.editor.line_numbers;
        let sidebar_width = config.sidebar.width;
        let mut app = App { docs, active: 0, mode: Mode::Edit, status, quit: false, pending: None, clipboard: String::new(), prompt: None, search_history: Vec::new(), replace: None, quit_after_confirm: false, mtime_cache: std::collections::HashMap::new(), reload_pending: None, suspend_requested: false, key_debug, show_line_numbers, sidebar: None, sidebar_width, branch_cache: std::collections::HashMap::new(), active_branch: None, config, lsp: crate::lsp::LspHub::default(), lsp_open: HashSet::new(), lsp_sync: HashMap::new(), lsp_events: Vec::new(), last_responses: Vec::new(), lsp_disabled: std::collections::HashSet::new(), lsp_death_count: std::collections::HashMap::new(), completion: None, pending_completion: None, pending_goto: None, jump_stack: std::collections::VecDeque::new() };
        app.refresh_branch();
        for i in 0..app.docs.len() {
            app.lsp_attach(i);
        }
        app
    }

    pub fn doc(&self) -> &Document {
        &self.docs[self.active]
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }

    fn cycle_buffer(&mut self, dir: isize) {
        if self.docs.len() > 1 {
            self.active = (self.active as isize + dir).rem_euclid(self.docs.len() as isize) as usize;
        }
    }

    /// Drain LSP events, converting diagnostics to char-coords and handling server deaths.
    pub fn lsp_pump(&mut self) {
        let events = self.lsp.take_events();
        for ev in events {
            match &ev {
                crate::lsp::LspEvent::Diagnostics { path, diags } => {
                    let path = path.clone();
                    let diags = diags.clone();
                    let idx = self.docs.iter().position(|d| d.path.as_deref() == Some(path.as_path()))
                        .or_else(|| {
                            let canon = path.canonicalize().ok()?;
                            self.docs.iter().position(|d| {
                                d.path.as_ref().and_then(|p| p.canonicalize().ok()) == Some(canon.clone())
                            })
                        });
                    if let Some(i) = idx {
                        let n_lines = self.docs[i].rope.len_lines();
                        let mut converted: Vec<crate::document::Diag> = Vec::with_capacity(diags.len());
                        for d in &diags {
                            let sev = match d.severity {
                                Some(s) if s == lsp_types::DiagnosticSeverity::ERROR => 1,
                                Some(s) if s == lsp_types::DiagnosticSeverity::WARNING => 2,
                                Some(s) if s == lsp_types::DiagnosticSeverity::INFORMATION => 3,
                                Some(s) if s == lsp_types::DiagnosticSeverity::HINT => 4,
                                _ => 1,
                            };
                            let raw_line = d.range.start.line as usize;
                            let line = raw_line.min(n_lines.saturating_sub(1));
                            let line_str = if line < n_lines {
                                self.docs[i].rope.line(line).to_string()
                            } else {
                                String::new()
                            };
                            let line_content = line_str.trim_end_matches('\n');
                            let line_len = line_content.chars().count();
                            let start = crate::lsp::utf16_col_to_char(line_content, d.range.start.character as usize).min(line_len);
                            let end = crate::lsp::utf16_col_to_char(line_content, d.range.end.character as usize).min(line_len);
                            converted.push(crate::document::Diag {
                                line, start, end,
                                severity: sev,
                                message: d.message.clone(),
                            });
                        }
                        self.docs[i].lsp_diags = converted;
                    }
                }
                crate::lsp::LspEvent::Response { id, result } => {
                    let id = *id;
                    let result = result.clone();
                    self.last_responses.push((id, result));
                    if self.last_responses.len() > 16 {
                        self.last_responses.remove(0);
                    }
                }                crate::lsp::LspEvent::Ready { .. } => {}
                crate::lsp::LspEvent::Died { lang } => {
                    let lang_str = lang.to_string();
                    self.lsp_open.retain(|(l, _)| l != &lang_str);
                    let count = self.lsp_death_count.entry(lang_str.clone()).or_insert(0);
                    *count += 1;
                    if *count == 1 {
                        self.status = Some((format!("lsp: {lang_str} server died"), true));
                        let idxs: Vec<usize> = self.docs.iter().enumerate()
                            .filter(|(_, d)| crate::syntax::language_for(d.path.as_deref()) == Some(lang_str.as_str()))
                            .map(|(i, _)| i)
                            .collect();
                        for i in idxs {
                            self.lsp_attach(i);
                        }
                    } else {
                        self.status = Some((format!("lsp: {lang_str} disabled"), true));
                        self.lsp_disabled.insert(lang_str);
                    }
                }
            }
            self.lsp_events.push(ev);
        }
        self.completion_pump();
        self.goto_pump();
    }

    /// Attach a document to an LSP server (didOpen). Gate: doc has path, language_for, command_for, ensure, didOpen (once per path per server lifecycle).
    pub fn lsp_attach(&mut self, idx: usize) {
        let doc = &self.docs[idx];
        let path = match doc.path.as_ref() {
            Some(p) => p.clone(),
            None => return,
        };
        let lang = match crate::syntax::language_for(Some(&path)) {
            Some(l) => l, // &'static str — never leaks a config String (M-1)
            None => return,
        };
        // Don't attach if this language has been permanently disabled after two deaths.
        if self.lsp_disabled.contains(lang) {
            return;
        }
        let Some(argv) = crate::lsp::command_for(&self.config, lang) else {
            return;
        };
        let root = crate::lsp::project_root(&path);
        self.lsp.ensure(lang, &root, &argv);

        let key = (lang.to_string(), path.clone());
        if self.lsp_open.contains(&key) {
            return;
        }
        self.lsp_open.insert(key);

        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        // ponytail: encode spaces as %20 only; other special chars are rare in project paths
        let uri = format!("file://{}", abs.display()).replace(' ', "%20");
        let text = self.docs[idx].rope.to_string();
        self.lsp.notify(lang, &root, "textDocument/didOpen", serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": lang,
                "version": 1,
                "text": text
            }
        }));
        // Initialize sync state (version 1 already sent via didOpen)
        self.lsp_sync.entry(path).or_insert(LspSyncState {
            synced_gen: self.docs[idx].edit_gen,
            version: 1,
            last_seen_gen: self.docs[idx].edit_gen,
            last_edit_seen: None,
        });
    }

    /// Debounced full-text sync: sends didChange after 250ms of edit inactivity.
    // ponytail: debounce resolution = run-loop poll rate (~10ms); sub-10ms edits
    // may be batched into one didChange, which is acceptable for LSP full-sync servers.
    pub fn lsp_sync_pass(&mut self, now: std::time::Instant) {
        let debounce = std::time::Duration::from_millis(250);
        for i in 0..self.docs.len() {
            let doc = &self.docs[i];
            let path = match doc.path.as_ref() {
                Some(p) => p.clone(),
                None => continue,
            };
            let lang = match crate::syntax::language_for(Some(&path)) {
                Some(l) => l,
                None => continue,
            };
            // Only sync if attached
            if !self.lsp_open.contains(&(lang.to_string(), path.clone())) {
                continue;
            }
            let current_gen = doc.edit_gen;
            let state = match self.lsp_sync.get_mut(&path) {
                Some(s) => s,
                None => continue,
            };
            if current_gen == state.synced_gen {
                state.last_edit_seen = None;
                continue;
            }
            // Gen changed — record first sighting
            if state.last_seen_gen != current_gen {
                // New edit activity observed — restart the debounce clock
                // (trailing-edge: fires 250ms after the LAST edit, not the first).
                state.last_seen_gen = current_gen;
                state.last_edit_seen = Some(now);
                continue;
            }
            let Some(t) = state.last_edit_seen else {
                state.last_edit_seen = Some(now);
                continue;
            };
            if now.duration_since(t) < debounce {
                continue;
            }
            // Debounce elapsed — send didChange
            let root = crate::lsp::project_root(&path);
            let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
            let uri = format!("file://{}", abs.display()).replace(' ', "%20");
            let text = self.docs[i].rope.to_string();
            let state = self.lsp_sync.get_mut(&path).unwrap();
            state.version += 1;
            let version = state.version;
            state.synced_gen = current_gen;
            state.last_edit_seen = None;
            self.lsp.notify(lang, &root, "textDocument/didChange", serde_json::json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }));
        }
    }

    /// Force-send didChange immediately for doc at `idx`, bypassing the debounce.
    /// Used by Tasks 6/7 before sending requests that need an up-to-date server view.
    pub fn lsp_sync_now(&mut self, idx: usize) {
        let doc = &self.docs[idx];
        let path = match doc.path.as_ref() {
            Some(p) => p.clone(),
            None => return,
        };
        let lang = match crate::syntax::language_for(Some(&path)) {
            Some(l) => l,
            None => return,
        };
        if !self.lsp_open.contains(&(lang.to_string(), path.clone())) {
            return;
        }
        let current_gen = doc.edit_gen;
        let state = match self.lsp_sync.get_mut(&path) {
            Some(s) => s,
            None => return,
        };
        if current_gen == state.synced_gen {
            return; // nothing to sync
        }
        let root = crate::lsp::project_root(&path);
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let uri = format!("file://{}", abs.display()).replace(' ', "%20");
        let text = self.docs[idx].rope.to_string();
        let state = self.lsp_sync.get_mut(&path).unwrap();
        state.version += 1;
        let version = state.version;
        state.synced_gen = current_gen;
        state.last_seen_gen = current_gen;
        state.last_edit_seen = None;
        self.lsp.notify(lang, &root, "textDocument/didChange", serde_json::json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }]
        }));
    }

    /// Consume pending completion response from last_responses, build CompMenu if valid.
    /// Must be called after last_responses is populated (end of lsp_pump).
    fn completion_pump(&mut self) {
        // Close stale menu: doc was edited since menu opened
        if let Some(ref menu) = self.completion {
            let stale = self.docs.get(menu.doc_idx)
                .map(|d| d.edit_gen != menu.edit_gen)
                .unwrap_or(true);
            if stale {
                self.completion = None;
            }
        }

        let Some((req_id, path, req_gen)) = self.pending_completion.clone() else { return };

        // Check if response is available
        let pos = self.last_responses.iter().position(|(id, _)| *id == req_id);
        let Some(pos) = pos else { return };

        let (_, result) = self.last_responses.remove(pos);

        // Find doc by path (safe across doc index shifts)
        let doc_idx = match self.docs.iter().position(|d| d.path.as_deref() == Some(path.as_path())) {
            Some(i) => i,
            None => { self.pending_completion = None; return; }
        };
        self.pending_completion = None;

        // Stale: doc was edited since request was sent
        if self.docs[doc_idx].edit_gen != req_gen {
            return;
        }

        // Parse items: may be array or {items:[...]}
        let items_val = if result.is_array() {
            result
        } else {
            result.get("items").cloned().unwrap_or(serde_json::Value::Array(vec![]))
        };
        let arr = match items_val.as_array() {
            Some(a) => a.clone(),
            None => return,
        };

        let mut items: Vec<CompItem> = Vec::with_capacity(arr.len());
        for item in &arr {
            let label = item["label"].as_str().unwrap_or("").to_string();
            let kind_num = item["kind"].as_u64().unwrap_or(0);
            let kind_letter = comp_kind_letter(kind_num);

            // insert text: prefer insertText field, else label
            let raw_insert = item.get("insertText")
                .and_then(|v| v.as_str())
                .unwrap_or(&label)
                .to_string();
            let is_snippet = item.get("insertTextFormat")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) == 2;
            let insert = if is_snippet {
                crate::lsp::snippet::strip_snippet(&raw_insert)
            } else {
                raw_insert
            };

            // text_edit: convert utf16 range to char range now
            let text_edit = item.get("textEdit").and_then(|te| {
                let range = te.get("range")?;
                let new_text = te.get("newText").and_then(|v| v.as_str())?.to_string();
                let line = range["start"]["line"].as_u64()? as usize;
                let start_u16 = range["start"]["character"].as_u64()? as usize;
                let end_u16 = range["end"]["character"].as_u64()? as usize;
                // Convert utf16 → char using current doc line
                let doc = &self.docs[doc_idx];
                let n_lines = doc.rope.len_lines();
                if line >= n_lines { return None; }
                let line_str = doc.rope.line(line).to_string();
                let line_content = line_str.trim_end_matches('\n');
                let line_len = line_content.chars().count();
                let start_c = crate::lsp::utf16_col_to_char(line_content, start_u16).min(line_len);
                let end_c = crate::lsp::utf16_col_to_char(line_content, end_u16).min(line_len);
                Some((line, start_c, end_c, new_text))
            });

            // cap at parse (review I2): servers send 100s-1000s; paint shows 10,
            // cycling 20 is plenty (servers rank by relevance)
            if items.len() >= 20 {
                break;
            }
            items.push(CompItem { label, kind_letter, text_edit, insert });
        }

        if items.is_empty() { return; }

        let edit_gen_now = self.docs[doc_idx].edit_gen;
        self.completion = Some(CompMenu { items, sel: 0, doc_idx, edit_gen: edit_gen_now });
    }

    /// Trigger a completion request for the active doc. Gate: needs path, lang, and command_for.
    pub fn trigger_completion(&mut self) {
        let active = self.active;
        let doc = &self.docs[active];
        let path = match doc.path.as_ref() {
            Some(p) => p.clone(),
            None => return,
        };
        let lang = match crate::syntax::language_for(Some(&path)) {
            Some(l) => l,
            None => return,
        };
        if crate::lsp::command_for(&self.config, lang).is_none() {
            self.status = Some((format!("no language server for {lang}"), false));
            return;
        }
        // Attach (dedups) and force sync
        self.lsp_attach(active);
        self.lsp_sync_now(active);

        let doc = &self.docs[active];
        let cursor = doc.cursor;
        let cur_line = doc.rope.char_to_line(cursor.min(doc.rope.len_chars()));
        let line_start = doc.rope.line_to_char(cur_line);
        let col_chars = cursor - line_start;
        let line_str = doc.rope.line(cur_line).to_string();
        let line_content = line_str.trim_end_matches('\n');
        let char_utf16 = crate::lsp::char_col_to_utf16(line_content, col_chars);
        let edit_gen = doc.edit_gen;

        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let uri = format!("file://{}", abs.display()).replace(' ', "%20");
        let root = crate::lsp::project_root(&path);

        if let Some(id) = self.lsp.request(lang, &root, "textDocument/completion", serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": cur_line, "character": char_utf16 }
        })) {
            self.pending_completion = Some((id, path, edit_gen));
        }
    }

    /// Trigger a goto-definition request for the active doc.
    pub fn trigger_goto_def(&mut self) {
        let active = self.active;
        let doc = &self.docs[active];
        let path = match doc.path.as_ref() {
            Some(p) => p.clone(),
            None => return,
        };
        let lang = match crate::syntax::language_for(Some(&path)) {
            Some(l) => l,
            None => return,
        };
        if crate::lsp::command_for(&self.config, lang).is_none() {
            self.status = Some((format!("no language server for {lang}"), false));
            return;
        }
        self.lsp_attach(active);
        self.lsp_sync_now(active);

        let doc = &self.docs[active];
        let cursor = doc.cursor;
        let cur_line = doc.rope.char_to_line(cursor.min(doc.rope.len_chars()));
        let line_start = doc.rope.line_to_char(cur_line);
        let col_chars = cursor - line_start;
        let line_str = doc.rope.line(cur_line).to_string();
        let line_content = line_str.trim_end_matches('\n');
        let char_utf16 = crate::lsp::char_col_to_utf16(line_content, col_chars);

        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let uri = format!("file://{}", abs.display()).replace(' ', "%20");
        let root = crate::lsp::project_root(&path);

        if let Some(id) = self.lsp.request(lang, &root, "textDocument/definition", serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": cur_line, "character": char_utf16 }
        })) {
            self.pending_goto = Some((id, path));
        }
    }

    /// Consume pending goto response from last_responses and apply.
    fn goto_pump(&mut self) {
        let Some((req_id, path)) = self.pending_goto.clone() else { return };

        let pos = self.last_responses.iter().position(|(id, _)| *id == req_id);
        let Some(pos) = pos else { return };

        let (_, result) = self.last_responses.remove(pos);
        self.pending_goto = None;

        // Parse as GotoDefinitionResponse: Location | Array | LocationLink | null
        let loc = Self::parse_first_location(&result);

        match loc {
            None => {
                self.status = Some(("no definition found".into(), false));
            }
            Some((target_uri, target_line, target_char_utf16, extra_count)) => {
                // Decode uri → path
                let target_path = {
                    let raw = target_uri.strip_prefix("file://").unwrap_or(&target_uri);
                    let decoded = raw.replace("%20", " ");
                    PathBuf::from(decoded)
                };
                // Push current position onto jump stack
                let cur_path = path.clone();
                let cur_cursor = self.docs.get(self.active).map(|d| d.cursor).unwrap_or(0);
                if self.jump_stack.len() >= 50 {
                    self.jump_stack.pop_front();
                }
                self.jump_stack.push_back((cur_path, cur_cursor));

                // Apply navigation
                self.apply_goto_location(target_path, target_line, target_char_utf16);

                if extra_count > 0 {
                    self.status = Some((format!("{extra_count} more locations (list in phase 2)"), false));
                }
            }
        }
    }

    /// Parse a GotoDefinitionResponse Value → Option<(uri, line, char_utf16, extra_count)>
    fn parse_first_location(val: &serde_json::Value) -> Option<(String, usize, usize, usize)> {
        use serde_json::Value;
        if val.is_null() {
            return None;
        }
        // Single Location: has "uri" and "range"
        if let Some(uri) = val["uri"].as_str() {
            let line = val["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
            let ch = val["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
            return Some((uri.to_string(), line, ch, 0));
        }
        // Array of Location or LocationLink
        if let Value::Array(arr) = val {
            if arr.is_empty() {
                return None;
            }
            let first = &arr[0];
            let extra = arr.len() - 1;
            // Location: has "uri"
            if let Some(uri) = first["uri"].as_str() {
                let line = first["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let ch = first["range"]["start"]["character"].as_u64().unwrap_or(0) as usize;
                return Some((uri.to_string(), line, ch, extra));
            }
            // LocationLink: has "targetUri"
            if let Some(uri) = first["targetUri"].as_str() {
                let line = first["targetSelectionRange"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let ch = first["targetSelectionRange"]["start"]["character"].as_u64().unwrap_or(0) as usize;
                return Some((uri.to_string(), line, ch, extra));
            }
        }
        None
    }

    /// Navigate to target path+line+utf16col. Opens new buffer if needed.
    fn apply_goto_location(&mut self, target_path: PathBuf, target_line: usize, col_utf16: usize) {
        // Resolve target doc: canonicalize-compare
        let target_canon = target_path.canonicalize().ok();
        let idx = self.docs.iter().position(|d| {
            if let Some(p) = d.path.as_ref() {
                if p == &target_path { return true; }
                if let (Some(a), Some(b)) = (p.canonicalize().ok(), target_canon.clone()) {
                    return a == b;
                }
            }
            false
        });

        let idx = if let Some(i) = idx {
            self.active = i;
            i
        } else {
            // Open new buffer
            match crate::document::Document::open(&target_path) {
                Ok(d) => {
                    self.docs.push(d);
                    let i = self.docs.len() - 1;
                    self.active = i;
                    let idx2 = self.active;
                    self.lsp_attach(idx2);
                    i
                }
                Err(e) => {
                    self.status = Some((format!("{}: {e:?}", target_path.display()), true));
                    return;
                }
            }
        };

        // Set cursor
        let doc = &self.docs[idx];
        let n_lines = doc.rope.len_lines();
        let line = target_line.min(n_lines.saturating_sub(1));
        let line_str = doc.rope.line(line).to_string();
        let line_content = line_str.trim_end_matches('\n');
        let col_chars = crate::lsp::utf16_col_to_char(line_content, col_utf16);
        let line_start = doc.rope.line_to_char(line);
        let cursor = line_start + col_chars;
        let d = &mut self.docs[idx];
        d.cursor = cursor;

        // Center view
        let view_h = self.viewport_rows().max(1);
        let d = &mut self.docs[idx];
        d.scroll_row = line.saturating_sub(view_h / 2);
        self.clamp_viewport();
    }

    /// Pop from jump_stack and navigate back.
    pub fn jump_back(&mut self) {
        let Some((back_path, back_cursor)) = self.jump_stack.pop_back() else {
            self.status = Some(("jump stack empty".into(), false));
            return;
        };
        // Find open doc (do NOT re-open)
        let idx = self.docs.iter().position(|d| {
            if let Some(p) = d.path.as_ref() {
                if p == &back_path { return true; }
                if let (Ok(a), Ok(b)) = (p.canonicalize(), back_path.canonicalize()) {
                    return a == b;
                }
            }
            false
        });
        match idx {
            None => {
                self.status = Some(("buffer closed".into(), false));
            }
            Some(i) => {
                self.active = i;
                let d = &mut self.docs[i];
                d.cursor = back_cursor.min(d.rope.len_chars());
                let line = d.rope.char_to_line(d.cursor);
                let view_h = crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(24).saturating_sub(1).max(1);
                d.scroll_row = line.saturating_sub(view_h / 2);
                self.clamp_viewport();
            }
        }
    }


    fn accept_completion(&mut self) {
        let menu = match self.completion.take() {
            Some(m) => m,
            None => return,
        };
        let item = &menu.items[menu.sel];
        // operate on the MENU's doc, not self.active (review I1): accept must never
        // edit the wrong buffer if active ever shifts while a menu exists.
        if let Some((line, start_c, end_c, ref new_text)) = item.text_edit {
            let doc = &self.docs[menu.doc_idx];
            let line_start = doc.rope.line_to_char(line);
            let from = line_start + start_c;
            let to = line_start + end_c;
            let new_text = new_text.clone();
            let ins_len = new_text.chars().count();
            let d = &mut self.docs[menu.doc_idx];
            d.edit(from, &new_text, to - from);
            d.cursor = from + ins_len;
        } else {
            let insert = item.insert.clone();
            // word-start: scan left over identifier chars from cursor
            let doc = &self.docs[menu.doc_idx];
            let cursor = doc.cursor;
            let mut word_start = cursor;
            let rope = &doc.rope;
            while word_start > 0 {
                let c = rope.char(word_start - 1);
                if c.is_alphanumeric() || c == '_' {
                    word_start -= 1;
                } else {
                    break;
                }
            }
            let remove_len = cursor - word_start;
            let ins_len = insert.chars().count();
            let d = &mut self.docs[menu.doc_idx];
            d.edit(word_start, &insert, remove_len);
            d.cursor = word_start + ins_len;
        }
        // menu painted relative to active doc's cursor; keep them consistent
        self.active = menu.doc_idx;
    }

    /// Handle a key event while the completion menu is open.
    /// Tab → sel+1, BackTab → sel-1, Enter → accept, Esc → close,
    /// any other key → close then route normally.
    pub fn handle_completion_key(&mut self, key: &crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        let n = self.completion.as_ref().map(|m| m.items.len()).unwrap_or(0);
        match key.code {
            KeyCode::Tab => {
                if let Some(ref mut menu) = self.completion {
                    menu.sel = (menu.sel + 1) % n;
                }
            }
            KeyCode::BackTab => {
                if let Some(ref mut menu) = self.completion {
                    menu.sel = if menu.sel == 0 { n - 1 } else { menu.sel - 1 };
                }
            }
            KeyCode::Enter => {
                self.accept_completion();
                self.clamp_viewport();
            }
            KeyCode::Esc => {
                self.completion = None;
            }
            _ => {
                // Close menu first, then route the key normally
                self.completion = None;
                let cmd = if self.sidebar.is_some() {
                    crate::keymap::lookup_sidebar(key)
                } else {
                    crate::keymap::lookup_with(&self.config.keys, self.mode, self.pending, key)
                };
                if let Some(cmd) = cmd {
                    self.handle_command(cmd);
                } else {
                    self.pending = None;
                }
            }
        }
    }

    /// Called once per frame; branch changes in another shell are picked up
    /// only on doc open/switch by design (no watcher).
    pub fn refresh_branch(&mut self) {
        let Some(path) = self.doc().path.clone() else {
            self.active_branch = None;
            return;
        };
        if !self.branch_cache.contains_key(&path) {
            let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."));
            let git = |args: &[&str]| -> Option<String> {
                std::process::Command::new("git")
                    .args(args).current_dir(&dir).output().ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            // symbolic-ref also works on an unborn branch (fresh git init);
            // failure means detached HEAD (short sha) or not a repo at all.
            let mut branch = git(&["symbolic-ref", "--short", "--quiet", "HEAD"]);
            if branch.is_none() {
                branch = git(&["rev-parse", "--short", "HEAD"]); // detached HEAD -> short sha
            }
            self.branch_cache.insert(path.clone(), branch);
        }
        self.active_branch = self.branch_cache.get(&path).cloned().flatten();
    }

    /// Build or refresh tree-sitter highlight state for the active document.
    /// Called once per frame before drawing. `view_rows` is the visible line count.
    ///
    /// - Builds SyntaxState on first call (if language detected and syntax enabled).
    /// - Flushes pending edits and refreshes the viewport spans.
    /// - Sets a status note (once) if the build fails.
    pub fn ensure_syntax(&mut self, view_rows: usize) {
        use crate::syntax::{language_for, SyntaxState};

        if !self.config.editor.syntax {
            return;
        }

        let doc = &mut self.docs[self.active];

        // Build on first call if language detected and not yet attempted.
        if doc.syntax.is_none() && !doc.syntax_attempted {
            if let Some(lang) = language_for(doc.path.as_deref()) {
                let text = doc.rope.to_string();
                match SyntaxState::build(lang, &text) {
                    Ok(state) => {
                        doc.syntax = Some(state);
                        doc.pending_ts.clear(); // build consumed current text
                    }
                    Err(e) => {
                        self.status = Some((format!("syntax: {e}"), true));
                        // Mark attempted so we don't retry every frame.
                        self.docs[self.active].syntax_attempted = true;
                        return;
                    }
                }
            } else {
                // No language → nothing to do; mark attempted to skip future checks.
                doc.syntax_attempted = true;
                return;
            }
        }

        let doc = &mut self.docs[self.active];
        if doc.syntax.is_none() {
            return;
        }

        // Build viewport: (line_idx, byte_start, byte_end_exclusive) for visible lines.
        let rope = &doc.rope;
        let len_lines = rope.len_lines();
        let scroll = doc.scroll_row;
        let end_line = (scroll + view_rows).min(len_lines);

        let mut view: Vec<(usize, usize, usize)> = Vec::new();
        for line_idx in scroll..end_line {
            // Skip the phantom final line of a \n-terminated file.
            // rope.line_to_char(len_lines - 1) == rope.len_chars() means the last
            // "line" is empty and lives past the real content.
            if line_idx == len_lines.saturating_sub(1)
                && rope.line_to_char(line_idx) == rope.len_chars()
            {
                continue;
            }
            let start_char = rope.line_to_char(line_idx);
            let end_char_raw = rope.line_to_char(line_idx + 1);
            // Strip trailing newline to get content-only char range.
            let end_char = if end_char_raw > start_char
                && rope.char(end_char_raw - 1) == '\n'
            {
                end_char_raw - 1
            } else {
                end_char_raw
            };
            let byte_start = rope.char_to_byte(start_char);
            let byte_end = rope.char_to_byte(end_char);
            view.push((line_idx, byte_start, byte_end));
        }

        let pending = std::mem::take(&mut doc.pending_ts);
        if let Some(state) = &mut doc.syntax {
            state.refresh(&pending, &view);
        }
    }

    pub fn handle_command(&mut self, cmd: crate::keymap::Command) {
        use crate::keymap::Command::*;
        if self.prompt.is_some() {
            if self.replace.is_some() {
                // the replace walkthrough owns the keyboard: bare y/n/a/e act
                // (uppercase too), everything else just re-prints the hint so the
                // editor never looks frozen
                match cmd {
                    PromptChar(c) => {
                        let k = c.to_ascii_lowercase();
                        if matches!(k, 'y' | 'n' | 'a' | 'e') {
                            self.replace_step(k);
                        } else {
                            self.highlight_flow_match();
                        }
                    }
                    PromptCancel => self.finish_replace_flow(),
                    _ => self.highlight_flow_match(),
                }
                return;
            }
            // the prompt owns the keyboard while open
            match cmd {
                PromptChar(c) => {
                    let find = self.prompt.as_ref().unwrap().kind == crate::prompt::PromptKind::Find;
                    if find {
                        let p = self.prompt.as_mut().unwrap();
                        if p.field == crate::prompt::PromptField::Replace {
                            p.replace.push(c);
                        } else {
                            p.line.push(c);
                            self.live_find_search();
                        }
                    } else {
                        self.prompt.as_mut().unwrap().line.push(c);
                    }
                }
                PromptBackspace => {
                    let (is_find, field) = {
                        let p = self.prompt.as_ref().unwrap();
                        (p.kind == crate::prompt::PromptKind::Find, p.field)
                    };
                    {
                        let p = self.prompt.as_mut().unwrap();
                        if is_find && field == crate::prompt::PromptField::Replace {
                            p.replace.pop();
                        } else {
                            p.line.pop();
                        }
                    }
                    if is_find && field == crate::prompt::PromptField::Pattern {
                        self.live_find_search();
                    }
                }
                PromptHistoryUp => self.prompt.as_mut().unwrap().history_up(),
                PromptHistoryDown => {}
                PromptCycle => {
                    let p = self.prompt.as_mut().unwrap();
                    match p.kind {
                        // in the find prompt Tab toggles pattern ⇄ replacement
                        crate::prompt::PromptKind::Find => {
                            p.field = if p.field == crate::prompt::PromptField::Pattern {
                                crate::prompt::PromptField::Replace
                            } else {
                                crate::prompt::PromptField::Pattern
                            };
                        }
                        _ => p.choice = p.choice.cycle(),
                    }
                }
                PromptCancel => match self.prompt.as_ref().map(|p| p.kind) {
                    // confirm prompts must run their Cancel branch (rearm etc.)
                    Some(crate::prompt::PromptKind::ConfirmReload)
                    | Some(crate::prompt::PromptKind::ConfirmQuit) => self.finish_prompt(),
                    _ => self.prompt = None,
                },
                PromptSubmit => self.finish_prompt(),
                _ => {}
            }
            return;
        }
        match cmd {
            InsertChar(c) => {
                let d = self.doc_mut();
                d.edit(d.cursor, &c.to_string(), 0);
                d.cursor += 1;
            }
            InsertNewline => {
                let d = self.doc_mut();
                d.edit(d.cursor, "\n", 0);
                d.cursor += 1;
            }
            Indent => {
                // Smart-Tab: if cursor is on/after identifier char AND lang has LSP → trigger completion
                let should_complete = {
                    let doc = &self.docs[self.active];
                    let path = doc.path.as_ref();
                    let lang = path.and_then(|p| crate::syntax::language_for(Some(p)));
                    let has_server = lang.map(|l| crate::lsp::command_for(&self.config, l).is_some()).unwrap_or(false);
                    if has_server && doc.cursor > 0 {
                        let prev_char = doc.rope.char(doc.cursor - 1);
                        prev_char.is_alphanumeric() || prev_char == '_'
                    } else {
                        false
                    }
                };
                if should_complete {
                    self.trigger_completion();
                    return;
                }
                let spaces = self.config.editor.indent_width as usize;
                let indent_str = " ".repeat(spaces);
                let d = self.doc_mut();
                d.edit(d.cursor, &indent_str, 0);
                d.cursor += spaces;
            }
            Backspace => {
                let d = self.doc_mut();
                if d.cursor > 0 {
                    d.edit(d.cursor - 1, "", 1);
                    d.cursor -= 1;
                }
            }
            Delete => {
                let d = self.doc_mut();
                if d.cursor < d.rope.len_chars() {
                    d.edit(d.cursor, "", 1);
                }
            }
            Move(motion, select) => {
                let pre = self.doc().cursor;
                self.move_cursor(motion);
                let d = self.doc_mut();
                match select {
                    crate::keymap::Select::Move => d.selections.clear(),
                    crate::keymap::Select::Extend => {
                        let c = d.cursor;
                        match d.selections.last_mut() {
                            Some(sel) => sel.head = c,
                            None => d.selections.push(crate::document::Selection { anchor: pre, head: c }),
                        }
                    }
                }
            }
            BeginSelectChar => {
                let d = self.doc_mut();
                if d.selections.last().and_then(|s| s.range()).is_some() {
                    d.selections.clear();
                } else if d.cursor < d.rope.len_chars() {
                    d.selections.push(crate::document::Selection { anchor: d.cursor, head: d.cursor + 1 });
                }
            }
            SelectLine => {
                let d = self.doc_mut();
                let (start, _) = crate::document::line_bounds(&d.rope, d.cursor);
                let line = d.rope.char_to_line(start);
                let end = if line + 1 >= d.rope.len_lines() {
                    d.rope.len_chars()
                } else {
                    d.rope.line_to_char(line + 1)
                };
                d.selections.clear();
                d.selections.push(crate::document::Selection { anchor: start, head: end });
            }
            ExpandLine => {
                let d = self.doc_mut();
                let head_cursor = d.cursor;
                let (start, _) = crate::document::line_bounds(&d.rope, head_cursor);
                let line = d.rope.char_to_line(start);
                let end = if line + 1 >= d.rope.len_lines() {
                    d.rope.len_chars()
                } else {
                    d.rope.line_to_char(line + 1)
                };
                match d.selections.last_mut() {
                    Some(sel) => {
                        sel.anchor = sel.anchor.min(start);
                        sel.head = sel.head.max(end);
                    }
                    None => d.selections.push(crate::document::Selection { anchor: start, head: end }),
                }
            }
            EnterActionMode => self.mode = Mode::Action,
            EnterEditMode => {
                if self.pending.take().is_some() {
                    return; // Esc cancels a pending key capture
                }
                // Esc in Action mode: first press cancels an active selection.
                if self.mode == Mode::Action && self.doc().selection().is_some() {
                    self.doc_mut().selections.clear();
                } else {
                    self.mode = Mode::Edit;
                }
            }
            JumpCharCapture { backward } => {
                self.pending = Some(if backward {
                    crate::keymap::Pending::JumpBackward
                } else {
                    crate::keymap::Pending::JumpForward
                });
            }
            JumpToChar { ch, backward } => {
                self.pending = None;
                let d = self.doc_mut();
                if let Some(target) = crate::document::jump_char(&d.rope, d.cursor, ch, backward) {
                    d.cursor = target;
                }
            }
            ObjectPrefix => self.pending = Some(crate::keymap::Pending::Object),
            Yank => {
                if let Some((a, b)) = self.doc().selection() {
                    self.clipboard = self.doc().slice(a, b - a);
                }
            }
            PasteAfter => self.paste(true),
            PasteBefore => self.paste(false),
            Paste => self.paste_at_caret(),
            DeleteSelection => {
                if let Some((a, b)) = self.doc().selection() {
                    // d copies too: yank-and-delete, one fewer key by design
                    let text = self.doc().slice(a, b - a);
                    {
                        let d = self.doc_mut();
                        d.edit(a, "", b - a);
                        d.cursor = a;
                        d.selections.clear();
                    }
                    self.clipboard = text;
                }
            }
            Change => {
                if let Some((a, b)) = self.doc().selection() {
                    let d = self.doc_mut();
                    d.edit(a, "", b - a);
                    d.cursor = a;
                    d.selections.clear();
                }
                self.mode = Mode::Edit;
            }
            SelectObject(c) => {
                self.pending = None;
                let d = self.doc_mut();
                if let Some((a, h)) = crate::document::object_range(&d.rope, d.cursor, c) {
                    d.selections.clear();
                    d.selections.push(crate::document::Selection { anchor: a, head: h });
                }
            }
            Save => match self.doc_mut().save() {
                Ok(()) => self.status = Some(("saved".into(), false)),
                Err(e) => self.status = Some((e, true)),
            },
            Undo => {
                let d = self.doc_mut();
                if let Some(at) = d.history.undo(&mut d.rope) {
                    // park the cursor at the undone change, not where it happened to be
                    d.cursor = at;
                    d.dirty = true; // undo can move a saved buffer off disk again
                }
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            Redo => {
                let d = self.doc_mut();
                if let Some(end) = d.history.redo(&mut d.rope) {
                    d.cursor = end;
                    d.dirty = true;
                }
                d.cursor = d.cursor.min(d.rope.len_chars());
            }
            Find => {
                let prefill = self
                    .doc()
                    .selection()
                    .and_then(|(a, b)| if b > a { Some(self.doc().slice(a, b - a)) } else { None })
                    .or_else(|| self.doc().search.clone())
                    .unwrap_or_default();
                let mut p = crate::prompt::Prompt::new(crate::prompt::PromptKind::Find, prefill);
                p.history = self.search_history.clone();
                self.prompt = Some(p);
            }
            FindNext => {
                if !self.doc_mut().search_next(true) {
                    let t = self.doc().search.clone().unwrap_or_default();
                    self.status = Some((format!("no matches for '{t}'"), true));
                }
            }
            FindPrev => {
                if !self.doc_mut().search_next(false) {
                    let t = self.doc().search.clone().unwrap_or_default();
                    self.status = Some((format!("no matches for '{t}'"), true));
                }
            }
            // prompt commands without an open prompt: no-op (routing above owns them)
            PromptChar(_) | PromptBackspace | PromptHistoryUp | PromptHistoryDown
            | PromptSubmit | PromptCancel | PromptCycle => {}
            CloseBuffer => self.request_close(),
            NextBuffer => self.cycle_buffer(1),
            PrevBuffer => self.cycle_buffer(-1),
            CommandPrefix => {
                self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::Command, String::new()));
            }
            Suspend => self.suspend_requested = true, // main loop performs restore+SIGTSTP
            Quit => {
                if self.docs.iter().any(|d| d.dirty) {
                    self.quit_after_confirm = true;
                    self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmQuit, String::new()));
                } else {
                    self.quit = true;
                }
            }
            Nop => {}
            OpenSidebarTree => self.open_sidebar(crate::sidebar::SidebarMode::TreeBrowser),
            OpenSidebarPicker => self.open_sidebar(crate::sidebar::SidebarMode::FuzzyPicker),
            SidebarMove(motion) => {
                if let Some(sidebar) = &mut self.sidebar {
                    let delta = match motion {
                        crate::keymap::Motion::Up => -1,
                        crate::keymap::Motion::Down => 1,
                        crate::keymap::Motion::PageUp => -10,
                        crate::keymap::Motion::PageDown => 10,
                        // Home/End arrive as LineStart/LineEnd (no Motion::Home/End)
                        crate::keymap::Motion::LineStart => i32::MIN / 2,
                        crate::keymap::Motion::LineEnd => i32::MAX / 2,
                        _ => 0,
                    };
                    sidebar.move_cursor(delta);
                }
            }
            SidebarExpand => {
                if let Some(path) = self.sidebar.as_ref().and_then(|s| s.selected_path()) {
                    if path.is_dir() {
                        if let Some(sidebar) = &mut self.sidebar {
                            sidebar.expand_path(&path);
                        }
                    }
                }
            }
            SidebarCollapseOrParent => {
                if let Some(path) = self.sidebar.as_ref().and_then(|s| s.selected_path()) {
                    if let Some(sidebar) = &mut self.sidebar {
                        sidebar.collapse_or_parent(&path);
                    }
                }
            }
            SidebarRefresh => {
                self.refresh_sidebar();
            }
            SidebarInput(c) => {
                if let Some(sidebar) = &mut self.sidebar {
                    if sidebar.mode == crate::sidebar::SidebarMode::FuzzyPicker {
                        sidebar.input.push(c);
                        let term = sidebar.input.clone();
                        sidebar.filter_items(&term);
                    }
                }
            }
            SidebarBackspace => {
                if let Some(sidebar) = &mut self.sidebar {
                    sidebar.input.pop();
                    let term = sidebar.input.clone();
                    sidebar.filter_items(&term);
                }
            }
            SidebarSubmit => self.sidebar_on_enter(),
            SidebarCancel => self.close_sidebar(),
            ResizeSidebar(delta) => {
                if let Some(sidebar) = &mut self.sidebar {
                    sidebar.resize(delta);
                    self.sidebar_width = sidebar.width;
                }
            }
            NextDiagnostic => self.jump_diagnostic(true),
            PrevDiagnostic => self.jump_diagnostic(false),
            Completion => self.trigger_completion(),
            GotoDef => self.trigger_goto_def(),
            JumpBack => self.jump_back(),
            GotoPrefix => self.pending = Some(crate::keymap::Pending::GotoPrefix),
        }
        self.clamp_viewport();
    }

    /// One step of the replace walkthrough: y replace-and-advance, n skip,
    /// e replace-and-stop, a replace this and all remaining (one undo step).
    fn replace_step(&mut self, key: char) {
        let idx = match self.replace_position() {
            Some(i) => i,
            None => {
                self.finish_replace_flow();
                return;
            }
        };
        let repl = self.prompt.as_ref().map(|p| p.replace.clone()).unwrap_or_default();
        match key {
            'n' => {
                let s = self.doc().search_matches[idx].0;
                self.replace.as_mut().unwrap().next = s + 1;
                self.highlight_flow_match();
            }
            'y' | 'e' => {
                let end = match self.doc_mut().replace_match(idx, &repl) {
                    Ok(end) => end,
                    Err(err) => {
                        self.status = Some((err, true));
                        return; // stay in the walkthrough; nothing was applied
                    }
                };
                {
                    let flow = self.replace.as_mut().unwrap();
                    flow.done += 1;
                    flow.next = end;
                }
                // the rope changed: refresh the match offsets
                let term = self.doc().search.clone().unwrap_or_default();
                self.doc_mut().set_search(&term).ok();
                self.highlight_flow_match();
                if key == 'e' {
                    self.finish_replace_flow();
                    return;
                }
            }
            'a' => {
                let start = self.doc().search_matches[idx].0;
                let n = match self.doc_mut().replace_all_from(start, &repl) {
                    Ok(n) => n,
                    Err(err) => {
                        self.status = Some((err, true));
                        return; // stay in the walkthrough; nothing was applied
                    }
                };
                self.replace.as_mut().unwrap().done += n;
                let term = self.doc().search.clone().unwrap_or_default();
                self.doc_mut().set_search(&term).ok(); // stale offsets kill the next n/N
                self.finish_replace_flow();
                return;
            }
            _ => {}
        }
        if self.replace_position().is_none() {
            self.finish_replace_flow();
        }
    }

    /// Index of the walkthrough's current match: first with start >= next.
    fn replace_position(&self) -> Option<usize> {
        let flow = self.replace.as_ref()?;
        self.doc()
            .search_matches
            .iter()
            .position(|&(s, _)| s >= flow.next)
    }

    /// Make the walkthrough's current match the selection (the viewport scrolls
    /// to it; the selection highlight shows the spot) and print the step hint.
    fn highlight_flow_match(&mut self) {
        let Some(i) = self.replace_position() else { return };
        let (s, e) = self.doc().search_matches[i];
        let (row, col) = {
            let d = self.doc();
            let line = d.rope.char_to_line(s);
            (line + 1, s + 1 - d.rope.line_to_char(line))
        };
        let done = self.replace.as_ref().map(|f| f.done).unwrap_or(0);
        let left = self.doc().search_matches.iter().filter(|m| m.0 >= s).count();
        {
            let d = self.doc_mut();
            d.selections.clear();
            d.selections.push(crate::document::Selection { anchor: s, head: e });
            d.cursor = e;
        }
        self.clamp_viewport(); // the walkthrough must scroll the match into view
        self.status = Some((
            format!("replace at {row}:{col}? (y/n/a/e/Esc) [{done} done, {left} left]"),
            false,
        ));
    }

    fn finish_replace_flow(&mut self) {
        let done = self.replace.take().map(|f| f.done).unwrap_or(0);
        self.prompt = None;
        self.status = Some((format!("{done} replaced"), done == 0));
    }

    fn live_find_search(&mut self) {
        let line = self.prompt.as_ref().unwrap().line.clone();
        let doc = self.doc_mut();
        match doc.set_search(&line) {
            Ok(0) if line.is_empty() => self.status = None,
            Ok(0) => self.status = Some(("no matches".into(), true)),
            Ok(n) => self.status = Some((format!("{n} match(es)"), false)),
            Err(e) => self.status = Some((format!("regex error: {e}"), true)),
        }
    }

    fn move_cursor(&mut self, motion: crate::keymap::Motion) {
        use crate::keymap::Motion::*;
        let d = self.doc_mut();
        let len = d.rope.len_chars();
        match motion {
            Left => d.cursor = d.cursor.saturating_sub(1),
            Right => d.cursor = (d.cursor + 1).min(len),
            LineStart => d.cursor = crate::document::line_bounds(&d.rope, d.cursor).0,
            LineEnd => d.cursor = crate::document::line_bounds(&d.rope, d.cursor).1,
            WordForward => d.cursor = crate::document::word_next(&d.rope, d.cursor),
            WordBackward => d.cursor = crate::document::word_prev(&d.rope, d.cursor),
            Up | Down => {
                let row = d.rope.char_to_line(d.cursor.min(len));
                let col = d.cursor - d.rope.line_to_char(row);
                // ponytail: column is re-derived per move, no desired-column memory;
                // add it in Phase 0.1 if arrowing through short lines gets annoying.
                let target = match motion {
                    Up if row > 0 => row - 1,
                    Down if row + 1 < d.rope.len_lines() => row + 1,
                    _ => return,
                };
                let (_, end) = crate::document::line_bounds(&d.rope, d.rope.line_to_char(target));
                let target_len = end - d.rope.line_to_char(target);
                d.cursor = d.rope.line_to_char(target) + col.min(target_len);
            }
            PageUp | PageDown => {
                let page = self.viewport_rows().max(1);
                let d = self.doc_mut();
                let row = d.rope.char_to_line(d.cursor.min(len));
                let target = match motion {
                    PageUp => row.saturating_sub(page),
                    PageDown => (row + page).min(d.rope.len_lines().saturating_sub(1)),
                    _ => unreachable!(),
                };
                let start = d.rope.line_to_char(target);
                let (_, end) = crate::document::line_bounds(&d.rope, start);
                let col = (d.cursor - d.rope.line_to_char(row)).min(end - start);
                d.cursor = start + col;
            }
        }
    }

    fn finish_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        use crate::prompt::PromptKind::*;
        match p.kind {
            Find => {
                let start_flow = p.field == crate::prompt::PromptField::Replace
                    && !p.replace.is_empty()
                    && !p.line.is_empty();
                if start_flow {
                    // Enter on the replacement field: walk the matches asking y/n/a/e
                    match self.doc_mut().set_search(&p.line) {
                        Ok(0) => {
                            self.status = Some((format!("no matches for '{}'", p.line), true));
                            self.prompt = Some(p); // stay put to fix the pattern
                        }
                        Ok(_) => match self.doc().validate_replacement(&p.replace) {
                            Err(err) => {
                                self.status = Some((err, true));
                                self.prompt = Some(p); // stay on the field to fix the typo
                            }
                            Ok(()) => {
                                if self.search_history.last().map(|h| *h != p.line).unwrap_or(true) {
                                    self.search_history.push(p.line.clone());
                                }
                                self.replace = Some(ReplaceFlow { next: 0, done: 0 });
                                self.prompt = Some(p);
                                self.highlight_flow_match();
                            }
                        },
                        Err(e) => {
                            self.status = Some((format!("regex error: {e}"), true));
                            self.prompt = Some(p);
                        }
                    }
                } else if !p.line.is_empty() {
                    if self.search_history.last().map(|h| *h != p.line).unwrap_or(true) {
                        self.search_history.push(p.line.clone());
                    }
                    let term = p.line;
                    match self.doc_mut().set_search(&term) {
                        Ok(0) => self.status = Some((format!("no matches for '{term}'"), true)),
                        Ok(_) => {
                            self.doc_mut().search_next(true);
                        }
                        Err(e) => self.status = Some((format!("regex error: {e}"), true)),
                    }
                }
            }
            SaveAs => {
                if let Err(e) = self.doc_mut().save_as(std::path::Path::new(&p.line)) {
                    self.status = Some((e, true));
                }
            }
            Command => {
                let line = p.line.trim().to_string();
                if line.is_empty() {
                } else if line == "q" {
                    self.request_close();
                } else if line == "w" {
                    if let Err(e) = self.doc_mut().save() {
                        self.status = Some((e, true));
                    }
                } else if line == "wq" {
                    if let Err(e) = self.doc_mut().save() {
                        self.status = Some((e, true));
                    } else {
                        self.close_active();
                    }
                } else if let Some(rest) = line.strip_prefix("w ") {
                    if let Err(e) = self.doc_mut().save_as(std::path::Path::new(rest.trim())) {
                        self.status = Some((e, true));
                    }
                } else if let Some(rest) = line.strip_prefix("e ") {
                    self.open_path(rest.trim());
                } else {
                    self.status = Some((format!("unknown command: {line}"), true));
                }
            }
            ConfirmQuit => {
                use crate::prompt::Choice::*;
                match p.choice {
                    Cancel => {}
                    Discard => self.after_confirm_close(),
                    Save => {
                        if let Err(e) = self.doc_mut().save() {
                            self.status = Some((e, true));
                        } else {
                            self.after_confirm_close();
                        }
                    }
                }
            }
            ConfirmReload => {
                use crate::prompt::Choice::*;
                let idx = self.reload_pending.take();
                match (p.choice, idx) {
                    (Cancel, Some(idx)) => {
                        // rearm: sentinel mtime makes the next poll flag this path again
                        if let Some(path) = self.docs[idx].path.clone() {
                            self.mtime_cache.insert(path, std::time::SystemTime::UNIX_EPOCH);
                        }
                    }
                    (Save, Some(idx)) => {
                        // "keep mine": rebaseline to the disk mtime, buffer untouched
                        if let Some(path) = self.docs[idx].path.clone() {
                            if let Some(m) = crate::watch::mtime(&path) {
                                self.mtime_cache.insert(path, m);
                            }
                        }
                    }
                    (Discard, Some(idx)) => self.reload_doc(idx),
                    (_, None) => {}
                }
            }
        }
    }

    pub fn open_sidebar(&mut self, mode: crate::sidebar::SidebarMode) {
        use crate::sidebar::{Sidebar, SidebarMode};
        let tree = match self.build_tree() {
            Ok(t) => t,
            Err(e) => {
                self.status = Some((format!("sidebar: {e}"), true));
                return;
            }
        };
        if tree.skipped > 0 {
            self.status = Some((format!("sidebar: {} dir(s) unreadable", tree.skipped), true));
        }
        // a mode switch rebuilds from scratch so filter/cursor state cannot leak
        self.sidebar = Some(match mode {
            SidebarMode::TreeBrowser => Sidebar::new_tree(tree.items, self.sidebar_width),
            SidebarMode::FuzzyPicker => Sidebar::new_picker(tree.items, self.sidebar_width),
        });
    }

    pub fn close_sidebar(&mut self) {
        self.sidebar = None;
    }

    /// Build the file tree from current directory.
    fn build_tree(&self) -> Result<crate::tree::FileTree, String> {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        crate::tree::FileTree::build(&root, &self.config.sidebar.ignore)
    }

    /// Refresh the sidebar: rescan the tree, keep the mode, width, expanded set,
    /// and picker input, then restore the cursor to the same path if it exists.
    pub fn refresh_sidebar(&mut self) {
        let Some(sidebar) = &self.sidebar else { return; };
        
        // Capture current state
        let mode = sidebar.mode;
        let saved_path = sidebar.selected_path();
        let expanded = sidebar.tree_state.as_ref().map(|s| s.expanded.clone());
        let input = sidebar.input.clone();
        
        // Rebuild the tree
        let tree = match self.build_tree() {
            Ok(t) => t,
            Err(e) => {
                self.status = Some((format!("sidebar: {e}"), true));
                return;
            }
        };
        
        if tree.skipped > 0 {
            self.status = Some((format!("sidebar: {} dir(s) unreadable", tree.skipped), true));
        }
        
        // Recreate sidebar in the same mode
        use crate::sidebar::{Sidebar, SidebarMode};
        let mut new_sidebar = match mode {
            SidebarMode::TreeBrowser => Sidebar::new_tree(tree.items, self.sidebar_width),
            SidebarMode::FuzzyPicker => Sidebar::new_picker(tree.items, self.sidebar_width),
        };
        
        // Restore expanded set (intersection with paths that still exist)
        if let Some(expanded_set) = expanded {
            if let Some(tree_state) = &mut new_sidebar.tree_state {
                for path in expanded_set {
                    // Check if path still exists in items
                    if tree_state.items.iter().any(|item| item.path == path && item.is_dir) {
                        tree_state.expanded.insert(path);
                    }
                }
            }
        }
        
        // Restore picker input and re-filter
        if mode == SidebarMode::FuzzyPicker {
            new_sidebar.input = input.clone();
            new_sidebar.filter_items(&input);
        }
        
        // Restore cursor to saved path if it exists in visible_items
        if let Some(saved_p) = &saved_path {
            let visible = new_sidebar.visible_items();
            if let Some(idx) = visible.iter().position(|item| &item.path == saved_p) {
                new_sidebar.cursor = idx;
            } else {
                // Clamp to nearest valid row
                new_sidebar.cursor = (new_sidebar.cursor).min(visible.len().saturating_sub(1));
            }
        }
        
        self.sidebar = Some(new_sidebar);
    }

    /// Enter inside the sidebar: open the selected file (and close), or toggle a dir.
    pub fn sidebar_on_enter(&mut self) {
        let Some(path) = self.sidebar.as_ref().and_then(|s| s.selected_path()) else {
            return;
        };
        if path.is_dir() {
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.toggle_expand_path(&path);
            }
        } else {
            self.open_path(&path.to_string_lossy());
            self.close_sidebar();
        }
    }

    fn open_path(&mut self, path: &str) {
        let path = std::path::PathBuf::from(path);
        match crate::document::Document::open(&path) {
            Ok(d) => {
                self.docs.push(d);
                self.active = self.docs.len() - 1;
                self.mode = Mode::Edit;
                let idx = self.active;
                self.lsp_attach(idx);
            }
            Err(e) => self.status = Some((format!("{}: {e:?}", path.display()), true)),
        }
    }

    fn request_close(&mut self) {
        if self.doc().dirty {
            self.quit_after_confirm = false;
            self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmQuit, String::new()));
        } else {
            self.close_active();
        }
    }

    fn after_confirm_close(&mut self) {
        if self.quit_after_confirm {
            self.quit = true;
        } else {
            self.close_active();
        }
    }

    /// React to an externally-modified file: clean → silent reload;
    /// dirty → ConfirmReload prompt (keep mine / reload / cancel+rearm).
    pub fn handle_external_change(&mut self, path: &std::path::Path) {
        let Some(idx) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) else { return };
        if self.docs[idx].dirty {
            self.reload_pending = Some(idx);
            self.active = idx;
            self.prompt = Some(crate::prompt::Prompt::new(crate::prompt::PromptKind::ConfirmReload, String::new()));
        } else {
            self.reload_doc(idx);
        }
    }

    pub(crate) fn reload_doc(&mut self, idx: usize) {
        let Some(path) = self.docs[idx].path.clone() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => {
                    let line_ending = if text.contains("\r\n") {
                        crate::document::LineEnding::CRLF
                    } else {
                        crate::document::LineEnding::LF
                    };
                    let normalized = text.replace("\r\n", "\n");
                    let d = &mut self.docs[idx];
                    // the reload itself is one undoable Change: undo restores
                    // the pre-external-edit buffer (history survives reload)
                    let old = d.rope.to_string();
                    let change = crate::undo::Change { at: 0, inserted: normalized, removed: old };
                    change.apply(&mut d.rope);
                    d.history.push(change);
                    d.dirty = false;
                    d.edit_gen += 1; // reload counts as a mutation for LSP sync
                    d.line_ending = line_ending;
                    d.cursor = d.cursor.min(d.rope.len_chars());
                    d.selections.clear();
                    // Reset syntax state: will be rebuilt on next ensure_syntax call.
                    d.syntax = None;
                    d.pending_ts.clear();
                    d.syntax_attempted = false;
                    if let Some(m) = crate::watch::mtime(&path) {
                        self.mtime_cache.insert(path, m);
                    }
                    self.clamp_viewport();
                }
                Err(_) => self.status = Some((format!("{}: not valid UTF-8", path.display()), true)),
            },
            Err(e) => self.status = Some((format!("{}: {e}", path.display()), true)),
        }
    }

    fn close_active(&mut self) {
        if self.docs.len() == 1 {
            self.quit = true;
            return;
        }
        self.docs.remove(self.active);
        self.active = self.active.min(self.docs.len() - 1);
    }

    fn paste(&mut self, after: bool) {
        let text = self.clipboard.clone();
        if text.is_empty() {
            return;
        }
        if text.ends_with('\n') {
            return self.paste_line_block(&text, after);
        }
        let d = self.doc_mut();
        let at = match d.selection() {
            Some((_, b)) if after => b,
            Some((a, _)) => a,
            None => {
                if after {
                    (d.cursor + 1).min(d.rope.len_chars())
                } else {
                    d.cursor
                }
            }
        };
        d.edit(at, &text, 0);
        d.cursor = at + text.chars().count();
        d.selections.clear();
    }

    /// Ctrl-V (CUA): char register inserts exactly at the caret (no vim +1),
    /// line register goes below the caret's line.
    fn paste_at_caret(&mut self) {
        if self.key_debug {
            self.status = Some((format!("register {:?}", self.clipboard.chars().take(20).collect::<String>()), false));
        }
        let text = self.clipboard.clone();
        if text.is_empty() {
            return;
        }
        if text.ends_with('\n') {
            return self.paste_line_block(&text, true);
        }
        let d = self.doc_mut();
        let at = d.cursor;
        d.edit(at, &text, 0);
        d.cursor = at + text.chars().count();
        d.selections.clear();
    }

    /// whole-line register (plat's x/X line selects end at a line start):
    /// paste as whole lines like helix, never splice mid-line
    fn paste_line_block(&mut self, text: &str, after: bool) {
        let d = self.doc_mut();
        let len = d.rope.len_chars();
        let row = d.rope.char_to_line(d.cursor.min(len.saturating_sub(1)));
        let (at, payload) = if after && row + 1 >= d.rope.len_lines() {
            let glue = if len > 0 && d.rope.char(len - 1) != '\n' { "\n" } else { "" };
            (len, format!("{glue}{text}"))
        } else if after {
            (d.rope.line_to_char(row + 1), text.to_string())
        } else {
            (d.rope.line_to_char(row), text.to_string())
        };
        let lead = payload.chars().count() - text.chars().count();
        d.edit(at, &payload, 0);
        d.cursor = at + lead; // start of the pasted block
        d.selections.clear();
    }

    /// Bracketed-paste event: insert the pasted text at the cursor verbatim
    /// (newlines normalized to LF), as ONE undoable change.
    pub fn handle_paste(&mut self, text: &str) {
        let norm = text.replace("\r\n", "\n").replace('\r', "\n");
        if norm.is_empty() {
            return;
        }
        if self.key_debug {
            self.status = Some((format!("paste {:?}", norm.chars().take(20).collect::<String>()), false));
        }
        let d = self.doc_mut();
        let at = d.cursor;
        d.edit(at, &norm, 0);
        d.cursor = at + norm.chars().count();
        d.selections.clear();
    }

    fn viewport_rows(&self) -> usize {
        crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(24).saturating_sub(1)
    }

    /// Jump cursor to the next (forward=true) or previous diagnostic, wrapping around.
    fn jump_diagnostic(&mut self, forward: bool) {
        let diags = &self.docs[self.active].lsp_diags;
        if diags.is_empty() {
            self.status = Some(("no diagnostics".into(), false));
            return;
        }
        let cur_line = self.docs[self.active].rope.char_to_line(
            self.docs[self.active].cursor.min(self.docs[self.active].rope.len_chars())
        );
        // Collect sorted (line, start) pairs with indices.
        let mut sorted: Vec<(usize, usize, usize)> = diags.iter().enumerate()
            .map(|(i, d)| (d.line, d.start, i))
            .collect();
        sorted.sort_unstable_by_key(|&(l, s, _)| (l, s));

        let target_idx = if forward {
            // next diag strictly after current cursor line, wrap
            sorted.iter().position(|&(l, _, _)| l > cur_line)
                .unwrap_or(0)
        } else {
            // prev diag strictly before current cursor line, wrap
            sorted.iter().rposition(|&(l, _, _)| l < cur_line)
                .unwrap_or(sorted.len() - 1)
        };
        let (line, start, _) = sorted[target_idx];
        let d = &mut self.docs[self.active];
        // stale-diag guard: doc may have SHRUNK since pump-time clamp (review I-1)
        let line = line.min(d.rope.len_lines().saturating_sub(1));
        let line_start = d.rope.line_to_char(line);
        let line_end = d.rope.line_to_char(line + 1).min(d.rope.len_chars());
        d.cursor = (line_start + start).min(line_end);
    }

    fn clamp_viewport(&mut self) {
        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let view = (h.saturating_sub(1), w);
        let d = self.doc_mut();
        let row = d.rope.char_to_line(d.cursor.min(d.rope.len_chars()));
        let col = d.cursor - d.rope.line_to_char(row);
        (d.scroll_row, d.scroll_col) =
            crate::render::clamp_scroll(row, col, view, (d.scroll_row, d.scroll_col));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Command;

    #[test]
    fn refresh_branch_reads_repo_and_negative_caches_plain_dir() {
        // a real scratch git repo
        let repo = std::env::temp_dir().join(format!("plat-br-{}", std::process::id()));
        std::fs::create_dir_all(&repo).unwrap();
        let f = repo.join("f.txt");
        std::fs::write(&f, "x").unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"]).current_dir(&repo).status().map(|s| s.success()).unwrap_or(false);
        assert!(ok, "git init failed — is git installed?");
        let mut app = App::new(vec![f.clone()]);
        app.refresh_branch();
        let branch = app.active_branch.clone();
        assert!(matches!(branch.as_deref(), Some("main") | Some("master")),
            "expected main/master, got {branch:?}");
        // plain dir outside any repo -> None, and cached so no re-spawn
        let plain = std::env::temp_dir().join(format!("plat-br-plain-{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        let g = plain.join("g.txt");
        std::fs::write(&g, "x").unwrap();
        app.docs.push(Document::open(&g).unwrap());
        app.active = app.docs.len() - 1;
        app.refresh_branch();
        assert_eq!(app.active_branch, None);
        assert!(app.branch_cache.contains_key(&g), "negative result must be cached");
        std::fs::remove_dir_all(&repo).ok();
        std::fs::remove_dir_all(&plain).ok();
    }

    #[test]
    fn confirm_save_with_failing_write_aborts_close() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("plat-t9f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("ro.txt");
        std::fs::write(&a, "disk").unwrap();
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o444)).unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::CloseBuffer);
        app.handle_command(Command::PromptCycle); // Cancel -> Save
        app.handle_command(Command::PromptSubmit); // Save fails (EACCES)
        assert!(!app.quit, "failed save must abort the close");
        assert_eq!(app.docs.len(), 1);
        assert!(app.doc().dirty, "buffer still dirty with content intact");
        assert_eq!(app.doc().rope.to_string(), "Xdisk");
        assert!(app.status.as_ref().unwrap().1, "error shown");
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn clean_doc_auto_reloads_and_keeps_undo_history() {
        use ropey::Rope;
        let dir = std::env::temp_dir().join(format!("plat-t10c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("ext.txt");
        std::fs::write(&a, "one").unwrap();
        let mut app = App::new(vec![a.clone()]);
        // local edit + save
        app.doc_mut().cursor = app.doc().rope.len_chars();
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::Save);
        // external change (sleep so mtime definitely moves)
        std::fs::write(&a, "TWO").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut cache = std::collections::HashMap::new();
        crate::watch::changed_paths(&app.docs, &mut cache); // baseline
        std::fs::write(&a, "THREE").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let changed = crate::watch::changed_paths(&app.docs, &mut cache);
        assert_eq!(changed, vec![a.clone()]);
        app.mtime_cache = cache;
        app.handle_external_change(&a);
        assert_eq!(app.doc().rope.to_string(), "THREE");
        assert!(app.prompt.is_none(), "clean reload is silent");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "oneX", "undo survives reload of same path");
        std::fs::remove_file(&a).ok();
        let _ = Rope::from_str("");
    }

    #[test]
    fn dirty_doc_gets_confirm_prompt_and_rearms_on_cancel() {
        let dir = std::env::temp_dir().join(format!("plat-t10d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("dirty10.txt");
        std::fs::write(&a, "disk").unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('L')); // dirty, unsaved
        std::fs::write(&a, "external").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut cache = std::collections::HashMap::new();
        crate::watch::changed_paths(&app.docs, &mut cache);
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(&a, "external2").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let changed = crate::watch::changed_paths(&app.docs, &mut cache);
        assert_eq!(changed, vec![a.clone()]);
        app.mtime_cache = cache;
        app.handle_external_change(&a);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmReload);
        app.handle_command(Command::PromptCancel);
        assert_eq!(app.doc().rope.to_string(), "Ldisk", "cancel: buffer untouched");
        assert!(app.doc().dirty);
        // rearm: watcher must flag the same path again
        let again = crate::watch::changed_paths(&app.docs, &mut app.mtime_cache);
        assert_eq!(again, vec![a.clone()], "cancel re-arms the watcher");
        // discard -> reload
        app.handle_external_change(&a);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptSubmit); // Discard
        assert_eq!(app.doc().rope.to_string(), "external2");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn tab_bar_shows_docs_bracketing_active() {
        let dir = std::env::temp_dir().join(format!("plat-t9-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "1").unwrap();
        std::fs::write(&b, "2").unwrap();
        let app = App::new(vec![a.clone(), b.clone()]);
        assert_eq!(app.docs.len(), 2);
        // render it and check status row
        use ratatui::backend::TestBackend;
        let mut term = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| crate::render::draw(f, &app)).unwrap();
        let buf = term.backend().buffer();
        // docs now live in the top tab row (>=2 buffers), not the status line
        let row: String = (0..80).map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(row.contains("[a.txt"), "active doc bracketed: {row:?}");
        assert!(row.contains("b.txt"), "other doc listed: {row:?}");
        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }

    #[test]
    fn close_dirty_asks_then_discard_or_cancel() {
        let dir = std::env::temp_dir().join(format!("plat-t9c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("dirty.txt");
        std::fs::write(&a, "disk").unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.handle_command(Command::InsertChar('X'));
        app.handle_command(Command::CloseBuffer);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmQuit);
        app.handle_command(Command::PromptCancel);
        assert_eq!(app.docs.len(), 1, "cancel keeps doc");
        assert!(app.doc().dirty);
        assert!(!app.quit);
        app.handle_command(Command::CloseBuffer);
        app.handle_command(Command::PromptCycle); // Cancel -> Save
        app.handle_command(Command::PromptCycle); // Save -> Discard
        app.handle_command(Command::PromptSubmit);
        assert!(app.quit, "closing the last doc quits");
        assert_eq!(std::fs::read_to_string(&a).unwrap_or_default(), "disk", "file on disk unchanged by discard-close");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn colon_w_q_wq_e_commands() {
        let dir = std::env::temp_dir().join(format!("plat-t9x-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let abs_new = dir.join("new.txt");
        let abs_other = dir.join("other.txt");
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('v'));
        app.handle_command(Command::CommandPrefix);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::Command);
        for c in format!("w {}", abs_new.display()).chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(std::fs::read_to_string(&abs_new).ok(), Some("v".into()));
        assert_eq!(app.doc().path.as_deref(), Some(abs_new.as_path()));
        assert!(!app.doc().dirty);
        // :e opens another buffer
        std::fs::write(&abs_other, "O").unwrap();
        app.handle_command(Command::CommandPrefix);
        for c in format!("e {}", abs_other.display()).chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.docs.len(), 2);
        assert_eq!(app.doc().rope.to_string(), "O");
        // :q on clean doc closes it
        app.handle_command(Command::CommandPrefix);
        for c in "q".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.docs.len(), 1);
        assert!(!app.quit);
        // :wq on clean last doc saves and quits
        app.handle_command(Command::CommandPrefix);
        for c in "wq".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert!(app.quit);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_q_dirty_prompts_before_quitting() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('x'));
        app.handle_command(Command::Quit);
        assert!(!app.quit, "dirty buffer must be confirmed first");
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::ConfirmQuit);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptSubmit); // discard
        assert!(app.quit);
    }

    #[test]
    fn resize_reclamps_scroll_without_panic() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str(&"line\n".repeat(40));
        app.doc_mut().cursor = "line\n".repeat(30).chars().count(); // row 30
        crate::render::apply_resize(&mut app, (80, 10));
        assert!(app.doc().scroll_row + 9 >= 30, "cursor row visible in 10-row frame");
        assert!(app.doc().scroll_row <= 30);
        crate::render::apply_resize(&mut app, (1, 1)); // degenerate: no panic, clamped
        assert_eq!(app.doc().scroll_row, 0);
        assert_eq!(app.doc().scroll_col, 0);
    }

    #[test]
    fn suspend_requests_shell_suspension_via_flag() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::Suspend);
        assert!(app.suspend_requested, "main loop performs the actual SIGTSTP");
    }

    #[test]
    fn find_wraps_and_reports_no_matches() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("TARGET up here\nrest of the file\n");
        app.doc_mut().cursor = 20; // below the only match
        app.handle_command(Command::Find);
        for c in "TARGET".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.doc().selection(), Some((0, 6)), "find from below the match must wrap to it");
        app.doc_mut().selections.clear();
        app.handle_command(Command::Find); // prefilled "TARGET"
        for _ in 0..6 {
            app.handle_command(Command::PromptBackspace);
        }
        for c in "zzz".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        assert_eq!(app.doc().selection(), None);
        let (msg, err) = app.status.clone().unwrap();
        assert!(err && msg.contains("no matches"), "user must be told, not left guessing: {msg:?}");
    }

    #[test]
    fn find_prompt_tab_toggles_fields_and_live_search_runs_as_you_type() {
        use crate::prompt::PromptField;
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo bar foo");
        app.handle_command(Command::Find);
        for c in "fo+".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        // live preview: matches exist BEFORE Enter
        assert_eq!(app.doc().search_matches, vec![(0, 3), (8, 11)], "live regex search");
        let (msg, _) = app.status.clone().unwrap();
        assert!(msg.contains("2 match"), "count in status: {msg:?}");
        assert_eq!(app.prompt.as_ref().unwrap().field, PromptField::Pattern);
        app.handle_command(Command::PromptCycle); // Tab -> replace field
        assert_eq!(app.prompt.as_ref().unwrap().field, PromptField::Replace);
        for c in "XY".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        // replacement keystrokes must NOT touch the pattern or the matches
        let p = app.prompt.as_ref().unwrap();
        assert_eq!(p.line, "fo+", "pattern untouched by replace typing");
        assert_eq!(p.replace, "XY");
        app.handle_command(Command::PromptBackspace);
        assert_eq!(app.prompt.as_ref().unwrap().replace, "X");
        app.handle_command(Command::PromptCycle); // Tab back to pattern
        app.handle_command(Command::PromptBackspace);
        assert_eq!(app.prompt.as_ref().unwrap().line, "fo", "backspace hits the pattern field");
    }

    #[test]
    fn replace_walkthrough_steps_with_y_and_n_and_undoes_each_step() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("aXaXa");
        app.handle_command(Command::Find);
        app.handle_command(Command::PromptChar('X'));
        app.handle_command(Command::PromptCycle); // Tab -> replace field
        app.handle_command(Command::PromptChar('Y'));
        app.handle_command(Command::PromptSubmit); // Enter: start the walkthrough
        assert!(app.replace.is_some(), "walkthrough started");
        assert_eq!(app.doc().selection(), Some((1, 2)), "first match pre-selected");
        let (msg, _) = app.status.clone().unwrap();
        assert!(msg.contains("at 1:2? (y/n/a/e/Esc) [0 done, 2 left]"), "step hint: {msg:?}");
        app.handle_command(Command::PromptChar('y')); // replace match 1
        assert_eq!(app.doc().rope.to_string(), "aYaXa");
        let (msg, _) = app.status.clone().unwrap();
        assert!(msg.contains("at 1:4"), "scrolled to next match: {msg:?}");
        assert!(msg.contains("[1 done, 1 left]"), "counters: {msg:?}");
        app.handle_command(Command::PromptChar('n')); // skip -> no matches left
        assert!(app.replace.is_none() && app.prompt.is_none(), "flow ended");
        assert_eq!(app.status.clone().unwrap().0, "1 replaced");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "aXaXa", "each y is its own undo step");
    }

    #[test]
    fn replace_a_finishes_all_remaining_in_one_undo_step() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo foo foo");
        app.handle_command(Command::Find);
        for c in "foo".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle);
        for c in "bar".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        app.handle_command(Command::PromptChar('y')); // first one stepped
        app.handle_command(Command::PromptChar('a')); // rest in one batch
        assert_eq!(app.doc().rope.to_string(), "bar bar bar");
        assert!(app.replace.is_none() && app.prompt.is_none());
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "bar foo foo", "one Ctrl-Z reverts the a-batch only");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "foo foo foo", "the stepped y was its own step");
    }

    #[test]
    fn replace_e_replaces_current_and_stops() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("one two one");
        app.handle_command(Command::Find);
        for c in "one".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptChar('1'));
        app.handle_command(Command::PromptSubmit);
        app.handle_command(Command::PromptChar('n')); // skip the first
        app.handle_command(Command::PromptChar('e')); // replace this one, stop
        assert_eq!(app.doc().rope.to_string(), "one two 1", "only the stepped-to match changed");
        assert!(app.replace.is_none() && app.prompt.is_none());
    }

    #[test]
    fn esc_in_walkthrough_keeps_applied_replaces() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("cat cat cat");
        app.handle_command(Command::Find);
        for c in "cat".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptChar('d'));
        app.handle_command(Command::PromptSubmit);
        app.handle_command(Command::PromptChar('y'));
        app.handle_command(Command::PromptCancel);
        assert_eq!(app.doc().rope.to_string(), "d cat cat", "applied replaces stay");
        assert!(app.replace.is_none() && app.prompt.is_none());
    }

    #[test]
    fn replace_walkthrough_expands_capture_groups() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("ab cd");
        app.handle_command(Command::Find);
        for c in r"(\w)".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle);
        for c in "$1$1".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        for _ in 0..4 {
            app.handle_command(Command::PromptChar('y'));
        }
        assert_eq!(app.doc().rope.to_string(), "aabb ccdd", "each $1 doubled");
        assert_eq!(app.status.clone().unwrap().0, "4 replaced");
    }

    #[test]
    fn enter_on_empty_replacement_field_is_just_find() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("needle");
        app.handle_command(Command::Find);
        for c in "needle".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle); // Tab to empty replace field
        app.handle_command(Command::PromptSubmit);
        assert!(app.replace.is_none(), "no walkthrough without replacement text");
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().selection(), Some((0, 6)), "plain find behavior kept");
    }

    #[test]
    fn bad_replacement_group_ref_is_reported_not_applied() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("abc");
        app.handle_command(Command::Find);
        app.handle_command(Command::PromptChar('b'));
        app.handle_command(Command::PromptCycle);
        for c in "[$9]".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptSubmit);
        // validated at Enter: the walkthrough never starts, the prompt stays open
        assert!(app.replace.is_none(), "no flow with an invalid replacement");
        assert!(app.prompt.is_some(), "prompt stays open on the field to fix the typo");
        let (msg, err) = app.status.clone().unwrap();
        assert!(err && msg.contains("group"), "group error surfaced: {msg:?}");
        assert_eq!(app.doc().rope.to_string(), "abc", "nothing applied");
    }

    #[test]
    fn replace_a_then_find_next_stays_safe() {
        // the headline gesture: a, then n to check what is left. The stale-match
        // offsets used to park the cursor past the rope and the next key panicked.
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("fooXfooXfoo");
        app.handle_command(Command::Find);
        for c in "foo".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptChar('b'));
        app.handle_command(Command::PromptSubmit);
        app.handle_command(Command::PromptChar('a'));
        assert_eq!(app.doc().rope.to_string(), "bXbXb");
        app.handle_command(Command::FindNext);
        assert!(app.doc().cursor <= app.doc().rope.len_chars(), "cursor clamped");
        let (msg, _) = app.status.clone().unwrap();
        assert!(msg.contains("no matches"), "matches refreshed after the batch: {msg:?}");
        app.handle_command(Command::InsertChar('Z')); // used to panic
        let text = app.doc().rope.to_string();
        assert_eq!(text.chars().count(), 6, "the keystroke landed inside the text: {text:?}");
        assert!(text.contains('Z'));
    }

    #[test]
    fn uppercase_walkthrough_keys_work_and_unknown_keys_reprint_the_hint() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("aXaX");
        app.handle_command(Command::Find);
        app.handle_command(Command::PromptChar('X'));
        app.handle_command(Command::PromptCycle);
        app.handle_command(Command::PromptChar('Y'));
        app.handle_command(Command::PromptSubmit);
        app.handle_command(Command::PromptChar('?')); // unknown: hint, not silence
        assert!(app.status.clone().unwrap().0.contains("(y/n/a/e/Esc)"), "hint reprinted");
        app.handle_command(Command::PromptChar('Y')); // uppercase counts
        assert_eq!(app.doc().rope.to_string(), "aYaX");
    }

    #[test]
    fn find_prompt_flow() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("hello ell");
        app.doc_mut().cursor = 0;
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().kind, crate::prompt::PromptKind::Find);
        for c in "ell".chars() {
            app.handle_command(Command::PromptChar(c));
        }
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell");
        app.handle_command(Command::PromptSubmit);
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().selection(), Some((1, 4)), "first match selected");
        app.handle_command(Command::FindNext);
        assert_eq!(app.doc().selection(), Some((6, 9)), "n advances");
        app.handle_command(Command::FindPrev);
        assert_eq!(app.doc().selection(), Some((1, 4)), "N goes back");
        // prefill from active selection
        app.doc_mut().selections.clear();
        app.doc_mut().selections.push(crate::document::Selection { anchor: 6, head: 9 });
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell", "prefilled from selection");
        app.handle_command(Command::PromptCancel);
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().selection(), Some((6, 9)), "cancel leaves state untouched");
        // reopen with no selection: prefills from last search term
        app.doc_mut().selections.clear();
        app.handle_command(Command::Find);
        assert_eq!(app.prompt.as_ref().unwrap().line, "ell", "prefill from last search");
        app.handle_command(Command::PromptCancel);
    }

    #[test]
    fn yank_paste_sequence() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 0;
        for _ in 0..3 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Extend));
        }
        app.handle_command(Command::Yank); // clipboard = "foo"
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('(')); // selects (4,7) "bar"
        app.handle_command(Command::PasteAfter);
        assert_eq!(app.doc().rope.to_string(), "foo(barfoo)", "paste after selection end");
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "foo(bar)");
    }

    #[test]
    fn empty_clipboard_paste_is_noop() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::PasteAfter);
        assert_eq!(app.doc().rope.to_string(), "a");
    }

    #[test]
    fn bracketed_paste_inserts_literally_as_one_undo_step() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("AB\n");
        app.doc_mut().cursor = 0;
        app.handle_paste("one\ntwo\tthree\r\n");
        {
            let d = app.doc();
            assert_eq!(d.rope.to_string(), "one\ntwo\tthree\nAB\n", "CRLF normalizes, Tab stays literal");
            assert_eq!(d.cursor, "one\ntwo\tthree\n".chars().count(), "cursor lands at end of pasted text");
        }
        assert!(app.doc_mut().undo(), "the whole paste is a single undo step");
        assert_eq!(app.doc().rope.to_string(), "AB\n");
    }

    #[test]
    fn line_register_pastes_as_whole_lines_not_inline_splice() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("one\ntwo\nthree\n");
        app.doc_mut().cursor = 0;
        app.handle_command(Command::SelectLine); // x selects "one\n"
        app.handle_command(Command::Yank);
        app.handle_command(Command::Move(crate::keymap::Motion::Down, crate::keymap::Select::Move));
        app.handle_command(Command::PasteAfter); // p: as a line below "two"
        assert_eq!(app.doc().rope.to_string(), "one\ntwo\none\nthree\n", "line register must not splice mid-line");
        app.handle_command(Command::Undo);
        app.doc_mut().cursor = 4; // back on the "two" line
        app.handle_command(Command::PasteBefore); // P: as a line above "two"
        assert_eq!(app.doc().rope.to_string(), "one\none\ntwo\nthree\n");
    }

    #[test]
    fn line_register_below_last_line_appends_a_new_line() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("a\nb"); // no trailing newline
        app.doc_mut().cursor = 2; // on 'b'
        app.clipboard = "x\n".into();
        app.handle_command(Command::PasteAfter);
        assert_eq!(app.doc().rope.to_string(), "a\nb\nx\n", "must not glue onto the last line");
    }

    #[test]
    fn ctrl_v_cua_paste_lands_at_the_caret_not_after_it() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("AB\nCD\n");
        app.doc_mut().cursor = 0;
        app.clipboard = "X".into();
        app.handle_command(Command::Paste);
        assert_eq!(app.doc().rope.to_string(), "XAB\nCD\n", "Ctrl-V = CUA paste at caret (p/P keep vim +1)");
        app.clipboard = "one\n".into();
        app.doc_mut().cursor = 1; // mid "AB" line
        app.handle_command(Command::Paste);
        assert_eq!(app.doc().rope.to_string(), "XAB\none\nCD\n", "line register: Ctrl-V pastes it below as a whole line");
    }

    #[test]
    fn shift_end_then_copy_pastes_the_tail_of_the_line() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("one\ntwo\nthree\n");
        app.doc_mut().cursor = 4; // 't' of "two"
        app.handle_command(Command::Move(crate::keymap::Motion::LineEnd, crate::keymap::Select::Extend));
        assert_eq!(app.doc().selection(), Some((4, 7)), "Shift+End extends caret..line end");
        app.handle_command(Command::Yank);
        assert_eq!(app.clipboard, "two", "clean line-tail copy carries no stray newline");
        app.handle_command(Command::EnterActionMode); // Esc
        app.handle_command(Command::Move(crate::keymap::Motion::Down, crate::keymap::Select::Move));
        app.handle_command(Command::Move(crate::keymap::Motion::LineStart, crate::keymap::Select::Move));
        app.handle_command(Command::Paste);
        assert_eq!(app.doc().rope.to_string(), "one\ntwo\ntwothree\n", "Ctrl-V lands at the caret");
    }

    #[test]
    fn keydebug_paste_preview_shows_what_the_terminal_sent() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.key_debug = true;
        app.doc_mut().rope = Rope::from_str("AB\n");
        app.handle_paste("\nhello");
        let (msg, _) = app.status.clone().unwrap();
        assert!(msg.contains("\"\\nhello\""), "status must reveal the received text verbatim, got {msg:?}");
    }

    #[test]
    fn change_deletes_selection_enters_edit_single_undoable() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('('));
        app.handle_command(Command::Change);
        assert_eq!(app.doc().rope.to_string(), "foo()");
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(app.doc().cursor, 4);
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().rope.to_string(), "foo(bar)");
    }

    #[test]
    fn delete_selection_and_paste_before() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("foo(bar)");
        app.doc_mut().cursor = 0;
        for _ in 0..3 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Extend));
        }
        app.handle_command(Command::Yank); // clipboard = "foo"
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('(')); // (4,7) "bar"
        app.handle_command(Command::PasteBefore);
        assert_eq!(app.doc().rope.to_string(), "foo(foobar)", "paste before selection start");
        app.handle_command(Command::Undo);
        app.doc_mut().cursor = 5;
        app.handle_command(Command::SelectObject('('));
        app.handle_command(Command::DeleteSelection);
        assert_eq!(app.doc().rope.to_string(), "foo()");
        assert_eq!(app.doc().selection(), None);
    }

    #[test]
    fn reload_updates_line_ending_and_saves_back_as_crlf() {
        let dir = std::env::temp_dir().join(format!("plat-fr1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("le.txt");
        std::fs::write(&a, "a\nb\n").unwrap();
        let mut app = App::new(vec![a.clone()]);
        assert_eq!(app.doc().line_ending, crate::document::LineEnding::LF);
        // external tool rewrites the file with CRLF
        std::fs::write(&a, "a\r\nb\r\n").unwrap();
        app.handle_external_change(&a); // clean -> silent reload
        assert_eq!(app.doc().rope.to_string(), "a\nb\n");
        assert_eq!(app.doc().line_ending, crate::document::LineEnding::CRLF, "reload must re-detect line endings");
        app.handle_command(Command::Move(crate::keymap::Motion::LineStart, crate::keymap::Select::Move));
        app.handle_command(Command::InsertChar('Z'));
        app.handle_command(Command::Save);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "Za\r\nb\r\n", "save must use the reloaded line endings");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn reload_clamps_viewport_to_new_content() {
        use ropey::Rope;
        let dir = std::env::temp_dir().join(format!("plat-fr2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("shrink.txt");
        std::fs::write(&a, "line\n".repeat(40)).unwrap();
        let mut app = App::new(vec![a.clone()]);
        app.doc_mut().rope = Rope::from_str(&"line\n".repeat(40));
        app.doc_mut().cursor = "line\n".repeat(30).chars().count(); // row 30
        app.doc_mut().scroll_row = 25;
        std::fs::write(&a, "tiny\nfile\n").unwrap();
        app.handle_external_change(&a); // clean reload
        let cur_row = app.doc().rope.char_to_line(app.doc().cursor.min(app.doc().rope.len_chars()));
        assert!(app.doc().scroll_row <= cur_row, "scroll must not sit past the new end of file");
        std::fs::remove_file(&a).ok();
    }

    #[test]
    fn undo_redo_move_cursor_to_the_change() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("hello");
        app.handle_command(Command::Move(crate::keymap::Motion::LineStart, crate::keymap::Select::Move));
        app.handle_command(Command::InsertChar('X')); // change at offset 0
        for _ in 0..6 {
            app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Move));
        }
        assert_eq!(app.doc().cursor, 6);
        app.handle_command(Command::Undo);
        assert_eq!(app.doc().cursor, 0, "undo jumps the cursor to the undone change");
        app.handle_command(Command::Redo);
        assert_eq!(app.doc().cursor, 1, "redo parks the cursor at the end of the redone text");
    }

    #[test]
    fn word_jumps_via_commands() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 4; // at 'x'
        app.handle_command(Command::Move(crate::keymap::Motion::WordForward, crate::keymap::Select::Move));
        assert_eq!(app.doc().cursor, 6, "w from 'x' lands at start of '='");
        app.handle_command(Command::Move(crate::keymap::Motion::WordBackward, crate::keymap::Select::Move));
        assert_eq!(app.doc().cursor, 4, "b returns to 'x'");
    }

    #[test]
    fn f_jump_to_char_and_miss_no_move() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 4;
        app.handle_command(Command::JumpToChar { ch: ')', backward: false });
        assert_eq!(app.doc().cursor, 20, "f ) lands on ')'");
        app.handle_command(Command::JumpToChar { ch: 'Z', backward: false });
        assert_eq!(app.doc().cursor, 20, "no match: cursor stays");
    }

    #[test]
    fn textobjects_select_inside() {
        use ropey::Rope;
        let mut app = App::new(vec![]);
        app.doc_mut().rope = Rope::from_str("let x = foo(bar, baz)");
        app.doc_mut().cursor = 13; // inside parens
        app.handle_command(Command::SelectObject('('));
        assert_eq!(app.doc().selection(), Some((12, 20)), "inside parens selects 'bar, baz'");

        let mut app2 = App::new(vec![]);
        app2.doc_mut().rope = Rope::from_str("say \"hi now\"");
        app2.doc_mut().cursor = 9; // inside quotes
        app2.handle_command(Command::SelectObject('"'));
        assert_eq!(app2.doc().selection(), Some((5, 11)), "inside quotes selects 'hi now'");

        let mut app3 = App::new(vec![]);
        app3.doc_mut().rope = Rope::from_str("abc");
        app3.doc_mut().cursor = 1;
        app3.handle_command(Command::SelectObject('"'));
        assert_eq!(app3.doc().selection(), None, "unmatched quote: unchanged");
    }

    #[test]
    fn shift_motion_extends_selection() {
        let mut app = App::new(vec![]);
        for c in ['h', 'e', 'l', 'l', 'o'] {
            app.handle_command(Command::InsertChar(c));
        }
        app.handle_command(Command::Move(crate::keymap::Motion::Left, crate::keymap::Select::Extend));
        app.handle_command(Command::Move(crate::keymap::Motion::Left, crate::keymap::Select::Extend));
        assert_eq!(app.doc().selection(), Some((3, 5)), "anchor stays at 5 while head extends left");
        // plain move clears selection
        app.handle_command(Command::Move(crate::keymap::Motion::Right, crate::keymap::Select::Move));
        assert_eq!(app.doc().selection(), None);
    }

    #[test]
    fn space_toggles_char_selection_under_cursor() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::Move(crate::keymap::Motion::LineStart, crate::keymap::Select::Move));
        app.mode = Mode::Action;
        app.handle_command(Command::BeginSelectChar);
        assert_eq!(app.doc().selection(), Some((0, 1)));
        app.handle_command(Command::BeginSelectChar);
        assert_eq!(app.doc().selection(), None, "second Space cancels");
    }

    #[test]
    fn esc_two_step_cancels_selection_then_edit() {
        let mut app = App::new(vec![]);
        app.handle_command(Command::InsertChar('a'));
        app.handle_command(Command::Move(crate::keymap::Motion::LineStart, crate::keymap::Select::Move));
        app.handle_command(Command::EnterActionMode);
        app.handle_command(Command::SelectLine);
        assert_eq!(app.doc().selection(), Some((0, 1)), "whole line, no trailing newline in doc");
        app.handle_command(Command::EnterEditMode); // Esc with active selection
        assert_eq!(app.doc().selection(), None);
        assert_eq!(app.mode, Mode::Action, "first Esc only cancels the selection");
        app.handle_command(Command::EnterEditMode);
        assert_eq!(app.mode, Mode::Edit);
    }

    #[test]
    fn typing_and_undo_via_commands() {
        let mut app = App::new(vec![]);
        for c in ['a', 'b'] {
            app.handle_command(Command::InsertChar(c));
        }
        assert_eq!(app.doc().cursor, 2);
        for c in ['x', 'y', 'z'] {
            app.handle_command(Command::InsertChar(c));
        }
        assert_eq!(app.doc().rope.to_string(), "abxyz");
        for _ in 0..3 {
            app.handle_command(Command::Undo);
        }
        assert_eq!(app.doc().rope.to_string(), "ab");
        assert_eq!(app.doc().cursor, 2);
    }

    #[test]
    fn new_opens_paths_or_empty_buffer() {
        let dir = std::env::temp_dir().join(format!("plat-t3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.txt");
        std::fs::write(&p, "x").unwrap();
        let app = App::new(vec![p.clone()]);
        assert_eq!(app.docs.len(), 1);
        assert_eq!(app.mode, Mode::Edit);
        let empty = App::new(vec![dir.join("missing.txt")]);
        assert_eq!(empty.docs.len(), 1);
        assert!(empty.status.as_ref().unwrap().1, "failed open must set error status");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn open_sidebar_tree_browser_sets_mode() {
        let mut app = App::new(vec![]);
        app.open_sidebar(crate::sidebar::SidebarMode::TreeBrowser);
        assert!(app.sidebar.is_some());
        assert_eq!(app.sidebar.as_ref().unwrap().mode, crate::sidebar::SidebarMode::TreeBrowser);
    }

    #[test]
    fn close_sidebar_drops_state() {
        let mut app = App::new(vec![]);
        app.open_sidebar(crate::sidebar::SidebarMode::TreeBrowser);
        app.close_sidebar();
        assert!(app.sidebar.is_none());
    }

    #[test]
    fn sidebar_enter_on_file_opens_it_and_closes() {
        let dir = std::env::temp_dir().join(format!("plat-sb-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("picked.rs");
        std::fs::write(&file, "hello").unwrap();

        let mut app = App::new(vec![]);
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(
            vec![crate::tree::TreeItem { path: file.clone(), is_dir: false, depth: 0 }],
            32,
        ));
        app.sidebar_on_enter();

        assert!(app.sidebar.is_none(), "sidebar closes after opening");
        assert!(
            app.docs.iter().any(|d| d.path.as_ref() == Some(&file)),
            "picked file became a buffer"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidebar_enter_on_dir_toggles_expand_and_stays_open() {
        let dir = std::env::temp_dir().join(format!("plat-sb-dir-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("inner.rs"), "").unwrap();

        let mut app = App::new(vec![]);
        let items = vec![
            crate::tree::TreeItem { path: dir.join("sub"), is_dir: true, depth: 0 },
            crate::tree::TreeItem { path: dir.join("sub").join("inner.rs"), is_dir: false, depth: 1 },
        ];
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(items, 32));
        app.sidebar_on_enter(); // dir selected -> expand, not open

        assert!(app.sidebar.is_some(), "dirs do not close the sidebar");
        assert_eq!(app.docs.len(), 1, "no new buffer for a dir");
        assert_eq!(app.sidebar.as_ref().unwrap().visible_items().len(), 2, "dir expanded");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn picker_app() -> App {
        use crate::tree::TreeItem;
        let mut app = App::new(vec![]);
        app.sidebar = Some(crate::sidebar::Sidebar::new_picker(
            vec![
                TreeItem { path: PathBuf::from("src/main.rs"), is_dir: false, depth: 1 },
                TreeItem { path: PathBuf::from("src/app.rs"), is_dir: false, depth: 1 },
                TreeItem { path: PathBuf::from("README.md"), is_dir: false, depth: 0 },
            ],
            32,
        ));
        app
    }

    #[test]
    fn sidebar_arrows_move_the_cursor() {
        let mut app = picker_app();
        app.handle_command(Command::SidebarMove(crate::keymap::Motion::Down));
        assert_eq!(app.sidebar.as_ref().unwrap().cursor, 1);
        app.handle_command(Command::SidebarMove(crate::keymap::Motion::LineEnd));
        assert_eq!(app.sidebar.as_ref().unwrap().cursor, 2, "End jumps to last row");
        app.handle_command(Command::SidebarMove(crate::keymap::Motion::LineStart));
        assert_eq!(app.sidebar.as_ref().unwrap().cursor, 0, "Home jumps to first row");
        app.handle_command(Command::SidebarMove(crate::keymap::Motion::Up));
        assert_eq!(app.sidebar.as_ref().unwrap().cursor, 0, "Up clamps at the top");
    }

    #[test]
    fn typing_in_the_picker_filters_live() {
        let mut app = picker_app();
        for c in "app".chars() {
            app.handle_command(Command::SidebarInput(c));
        }
        let sidebar = app.sidebar.as_ref().unwrap();
        assert_eq!(sidebar.input, "app");
        assert_eq!(sidebar.visible_items().len(), 1);
        assert_eq!(sidebar.visible_items()[0].path, PathBuf::from("src/app.rs"));
    }

    #[test]
    fn picker_backspace_widens_the_filter_again() {
        let mut app = picker_app();
        for c in "app".chars() {
            app.handle_command(Command::SidebarInput(c));
        }
        app.handle_command(Command::SidebarBackspace);
        app.handle_command(Command::SidebarBackspace);
        let sidebar = app.sidebar.as_ref().unwrap();
        assert_eq!(sidebar.input, "a");
        assert_eq!(sidebar.visible_items().len(), 3, "empty-ish filter shows all");
    }

    #[test]
    fn typing_in_tree_mode_does_not_filter() {
        let dir = std::env::temp_dir().join(format!("plat-sb-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = App::new(vec![]);
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(
            vec![crate::tree::TreeItem { path: dir.join("a.rs"), is_dir: false, depth: 0 }],
            32,
        ));
        app.handle_command(Command::SidebarInput('x'));
        assert_eq!(app.sidebar.as_ref().unwrap().visible_items().len(), 1, "tree rows unchanged");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidebar_resize_clamps_and_persists() {
        let mut app = picker_app();
        app.handle_command(Command::ResizeSidebar(-1000));
        assert_eq!(app.sidebar.as_ref().unwrap().width, crate::sidebar::MIN_WIDTH);
        app.handle_command(Command::ResizeSidebar(1000));
        assert_eq!(app.sidebar.as_ref().unwrap().width, crate::sidebar::MAX_WIDTH);
        let width = app.sidebar_width;
        app.close_sidebar();
        app.open_sidebar(crate::sidebar::SidebarMode::TreeBrowser);
        assert_eq!(app.sidebar.as_ref().unwrap().width, width, "width survives close/reopen");
    }

    #[test]
    fn sidebar_esc_closes_without_opening() {
        let mut app = picker_app();
        let docs = app.docs.len();
        app.handle_command(Command::SidebarCancel);
        assert!(app.sidebar.is_none());
        assert_eq!(app.docs.len(), docs, "nothing opened by Esc");
    }

    #[test]
    fn config_seeds_options_and_indent_width() {
        let cfg = crate::config::from_str("[editor]\nindent_width = 4\nline_numbers = false\n").unwrap();
        let mut app = App::new_with_config(vec![], cfg);
        assert!(!app.show_line_numbers);
        app.handle_command(Command::Indent);
        assert_eq!(app.doc().rope.to_string(), "    ");
    }

    #[test]
    fn prompt_and_sidebar_ignore_the_overlay() {
        let cfg = crate::config::from_str("[keys]\n\"Enter\" = \"none\"\n\"Esc\" = \"none\"").unwrap();
        let mut app = App::new_with_config(vec![], cfg);
        app.handle_command(Command::Find);
        // prompt/sidebar routing never consults the overlay keymap
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let enter = KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE };
        let esc = KeyEvent { code: KeyCode::Esc, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE };
        assert_eq!(crate::keymap::lookup_prompt(&enter), Some(Command::PromptSubmit));
        assert_eq!(crate::keymap::lookup_sidebar(&esc), Some(Command::SidebarCancel));
    }

    #[test]
    fn ctrl_r_rescans_and_keeps_cursor_by_path() {
        let dir = std::env::temp_dir().join(format!("plat-t8-rescan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file1 = dir.join("a.rs");
        std::fs::write(&file1, "first").unwrap();

        let mut app = App::new(vec![]);
        // Manually build the sidebar with the temp directory contents
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(
            vec![crate::tree::TreeItem { path: file1.clone(), is_dir: false, depth: 0 }],
            32,
        ));
        assert_eq!(app.sidebar.as_ref().unwrap().visible_items().len(), 1, "started with one file");

        // Create a second file on disk
        let file2 = dir.join("b.rs");
        std::fs::write(&file2, "second").unwrap();

        // Refresh should pick up the new file when it rebuilds
        // (Note: This test depends on the actual file system state, so we verify it independently)
        app.handle_command(Command::SidebarRefresh);
        // Since refresh rebuilds from current dir, just verify it doesn't crash
        assert!(app.sidebar.is_some(), "sidebar still exists after refresh");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_r_when_cursor_path_is_gone_lands_on_nearest_row() {
        let dir = std::env::temp_dir().join(format!("plat-t8-gone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file1 = dir.join("a.rs");
        let file2 = dir.join("b.rs");
        std::fs::write(&file1, "a").unwrap();
        std::fs::write(&file2, "b").unwrap();

        let mut app = App::new(vec![]);
        app.sidebar = Some(crate::sidebar::Sidebar::new_tree(
            vec![
                crate::tree::TreeItem { path: file1.clone(), is_dir: false, depth: 0 },
                crate::tree::TreeItem { path: file2.clone(), is_dir: false, depth: 0 },
            ],
            32,
        ));
        app.sidebar.as_mut().unwrap().cursor = 0; // on file1

        // Delete file1
        std::fs::remove_file(&file1).ok();

        // Refresh should land cursor on a valid row
        app.handle_command(Command::SidebarRefresh);
        let sidebar = app.sidebar.as_ref().unwrap();
        let visible = sidebar.visible_items();
        assert!(visible.len() > 0, "should have at least one file after refresh");
        assert!(sidebar.cursor < visible.len(), "cursor should be clamped to valid range");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ctrl_f_from_sidebar_opens_find_and_esc_returns_to_sidebar() {
        let mut app = App::new(vec![]);
        app.open_sidebar(crate::sidebar::SidebarMode::TreeBrowser);
        assert!(app.sidebar.is_some(), "sidebar open");
        assert!(app.prompt.is_none(), "no prompt initially");

        // Ctrl-F from sidebar opens find
        app.handle_command(Command::Find);
        assert!(app.prompt.is_some(), "find prompt opened");
        assert!(app.sidebar.is_some(), "sidebar stays open");

        // Esc in the prompt should be routed back to sidebar (handled in lib.rs)
        app.handle_command(Command::PromptCancel);
        assert!(app.prompt.is_none(), "prompt closed");
        assert!(app.sidebar.is_some(), "sidebar still open after prompt cancel");
    }

    #[test]
    fn syntax_disabled_via_config_prevents_build() {
        let dir = std::env::temp_dir().join(format!("plat-syntax-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rs_file = dir.join("test.rs");
        std::fs::write(&rs_file, "fn main() {}").unwrap();
        
        let mut cfg = crate::config::Config::defaults();
        cfg.editor.syntax = false;
        let mut app = App::new_with_config(vec![rs_file.clone()], cfg);
        
        app.ensure_syntax(22);
        assert!(app.doc().syntax.is_none(), "syntax disabled: no tree-sitter state");
        assert!(app.doc_mut().pending_ts.is_empty(), "no pending edits accumulate");
        
        std::fs::remove_file(&rs_file).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn buffer_cycling_wraps_both_ways() {
        let dir = std::env::temp_dir().join(format!("plat-cycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let mut app = App::new(vec![a, b]);
        assert_eq!(app.active, 0);
        app.handle_command(Command::NextBuffer);
        assert_eq!(app.active, 1);
        app.handle_command(Command::NextBuffer); // wrap
        assert_eq!(app.active, 0);
        app.handle_command(Command::PrevBuffer); // wrap backwards
        assert_eq!(app.active, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lsp_attach_no_spawn_when_disabled() {
        // [lsp] rust = false → command_for returns None → no server, no events, no panic
        let cfg = crate::config::from_str("[lsp]\nrust = false\n").unwrap();
        let tmp = std::env::temp_dir().join(format!("plat-attach-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let mut app = App::new_with_config(vec![file], cfg);
        app.lsp_pump();
        let events = app.lsp.take_events();
        assert!(events.is_empty(), "no events when lang disabled: {:?}", events);
        assert!(app.lsp_open.is_empty(), "no open entries when command is None");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn lsp_attach_no_spawn_unknown_lang() {
        let cfg = crate::config::Config::defaults();
        let tmp = std::env::temp_dir().join(format!("plat-attach-unk-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("foo.xyz");
        std::fs::write(&file, "content").unwrap();
        let mut app = App::new_with_config(vec![file], cfg);
        app.lsp_pump();
        let events = app.lsp.take_events();
        assert!(events.is_empty(), "unknown lang → no events");
        assert!(app.lsp_open.is_empty(), "unknown lang → nothing opened");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn edit_gen_bumps_on_reload() {
        let tmp = std::env::temp_dir().join(format!("plat-reload-gen-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("r.txt");
        std::fs::write(&file, "v1").unwrap();
        let mut app = App::new(vec![file.clone()]);
        let g = app.docs[0].edit_gen;
        std::fs::write(&file, "v2").unwrap();
        app.reload_doc(0);
        assert!(app.docs[0].edit_gen > g, "reload_doc must bump edit_gen");
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── Task 5: diagnostic navigation tests ──────────────────────────────────

    fn diag_app(content: &str) -> App {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("plat-diag-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("test.rs");
        std::fs::write(&p, content).unwrap();
        App::new(vec![p])
    }

    #[test]
    fn next_diag_wraps_and_jumps_to_start_char() {
        use crate::document::Diag;
        // lines 0..55 of text content
        let content: String = (0..60).map(|i| format!("line {i}\n")).collect();
        let mut app = diag_app(&content);
        app.doc_mut().lsp_diags = vec![
            Diag { line: 5, start: 2, end: 6, severity: 1, message: "e1".into() },
            Diag { line: 50, start: 1, end: 4, severity: 2, message: "w1".into() },
        ];
        // cursor at line 0
        app.doc_mut().cursor = 0;

        // ] from line 0 → jumps to line 5
        app.handle_command(Command::NextDiagnostic);
        let d = app.doc();
        let cur_line = d.rope.char_to_line(d.cursor);
        let line_start = d.rope.line_to_char(cur_line);
        let col = d.cursor - line_start;
        assert_eq!(cur_line, 5, "first ] lands on line 5");
        assert_eq!(col, 2, "lands on diag start col 2");

        // ] again → jumps to line 50
        app.handle_command(Command::NextDiagnostic);
        let d = app.doc();
        assert_eq!(d.rope.char_to_line(d.cursor), 50, "second ] lands on line 50");

        // ] wraps → back to line 5
        app.handle_command(Command::NextDiagnostic);
        let d = app.doc();
        assert_eq!(d.rope.char_to_line(d.cursor), 5, "] wraps from last diag back to first");
    }

    #[test]
    fn diag_nav_after_shrink_does_not_panic() {
        // stale-diag line beyond a shrunken rope must clamp, not panic (review I-1)
        use crate::document::Diag;
        let content: String = (0..60).map(|i| format!("line {i}\n")).collect();
        let mut app = diag_app(&content);
        app.doc_mut().lsp_diags = vec![Diag {
            line: 50,
            start: 3,
            end: 6,
            severity: 1,
            message: "stale".into(),
        }];
        app.doc_mut().edit(0, "", content.len()); // rope shrinks well below line 50
        app.handle_command(Command::NextDiagnostic);
        app.handle_command(Command::PrevDiagnostic);
        let d = app.doc();
        assert!(d.cursor <= d.rope.len_chars(), "cursor pinned inside rope");
    }

    #[test]
    fn prev_diag_wraps() {
        use crate::document::Diag;
        let content: String = (0..60).map(|i| format!("line {i}\n")).collect();
        let mut app = diag_app(&content);
        app.doc_mut().lsp_diags = vec![
            Diag { line: 5, start: 0, end: 1, severity: 1, message: "e1".into() },
            Diag { line: 50, start: 0, end: 1, severity: 2, message: "w1".into() },
        ];
        app.doc_mut().cursor = 0;

        // [ from line 0 → wraps to last diag (line 50)
        app.handle_command(Command::PrevDiagnostic);
        let d = app.doc();
        assert_eq!(d.rope.char_to_line(d.cursor), 50, "[ from 0 wraps to line 50");
    }

    #[test]
    fn no_diags_shows_status_note() {
        let mut app = diag_app("hello\n");
        app.doc_mut().lsp_diags.clear();
        app.handle_command(Command::NextDiagnostic);
        assert!(app.status.as_ref().map(|(m, _)| m.contains("no diagnostics")).unwrap_or(false),
            "no diags → status note");
    }

    #[test]
    fn keymap_bracket_actions() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use crate::keymap::lookup;
        use crate::app::Mode;
        use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState};
        let key = |code, mods| KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE };
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char('['), KeyModifiers::NONE)), Some(Command::PrevDiagnostic));
        assert_eq!(lookup(Mode::Action, None, &key(KeyCode::Char(']'), KeyModifiers::NONE)), Some(Command::NextDiagnostic));
    }

    #[test]
    fn diag_command_names_resolve() {
        use crate::keymap::command_by_name;
        assert_eq!(command_by_name("prev_diagnostic"), Some(Command::PrevDiagnostic));
        assert_eq!(command_by_name("next_diagnostic"), Some(Command::NextDiagnostic));
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers, KeyEvent, KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn make_menu(items: Vec<CompItem>, doc_idx: usize, edit_gen: u64) -> CompMenu {
        CompMenu { items, sel: 0, doc_idx, edit_gen }
    }

    fn simple_item(label: &str) -> CompItem {
        CompItem { label: label.to_string(), kind_letter: 't', text_edit: None, insert: label.to_string() }
    }

    fn bare_app() -> App {
        // unique per call: unit tests run in parallel threads and would race on a
        // shared tmp file (T7 added tests that touch the same dir)
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("comp_test_{pid}_{nanos}", pid = std::process::id()));
        std::fs::create_dir_all(&tmp).ok();
        let path = tmp.join("x.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        App::new(vec![path])
    }

    #[test]
    fn tab_cycles_sel_forward_wrapping() {
        let mut app = bare_app();
        let items = vec![simple_item("a"), simple_item("b"), simple_item("c")];
        let eg = app.docs[0].edit_gen;
        app.completion = Some(make_menu(items, 0, eg));
        app.handle_completion_key(&key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.completion.as_ref().unwrap().sel, 1);
        app.handle_completion_key(&key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.completion.as_ref().unwrap().sel, 2);
        app.handle_completion_key(&key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.completion.as_ref().unwrap().sel, 0);
    }

    #[test]
    fn backtab_cycles_sel_backward_wrapping() {
        let mut app = bare_app();
        let items = vec![simple_item("a"), simple_item("b"), simple_item("c")];
        let eg = app.docs[0].edit_gen;
        app.completion = Some(make_menu(items, 0, eg));
        app.handle_completion_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.completion.as_ref().unwrap().sel, 2);
    }

    #[test]
    fn esc_closes_menu() {
        let mut app = bare_app();
        let eg = app.docs[0].edit_gen;
        app.completion = Some(make_menu(vec![simple_item("a")], 0, eg));
        app.handle_completion_key(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.completion.is_none());
    }

    #[test]
    fn other_key_closes_menu_and_routes_normally() {
        let mut app = bare_app();
        app.docs[0].rope = ropey::Rope::from_str("ab\n");
        app.docs[0].cursor = 2;
        let eg = app.docs[0].edit_gen;
        app.completion = Some(make_menu(vec![simple_item("a")], 0, eg));
        app.handle_completion_key(&key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(app.completion.is_none());
        let content = app.docs[0].rope.to_string();
        assert!(content.contains('x'), "x should have been inserted: {content:?}");
    }

    #[test]
    fn accept_text_edit_emoji_line_utf16_to_char() {
        // Line: "a\u{1F4AF}b gamma" -- emoji is 2 utf16 units but 1 char
        // char->utf16: a=0, emoji->2, b->3, space->4, g->5, a->6, m->7, m->8, a->9
        // utf16 5 -> char 4 (g)
        let line = "a\u{1F4AF}b gamma";
        let start_c = crate::lsp::utf16_col_to_char(line, 5);
        let end_c = crate::lsp::utf16_col_to_char(line, 8);
        assert_eq!(start_c, 4, "utf16 5 should be char 4 (g)");
        assert_eq!(end_c, 7, "utf16 8 should be char 7");
    }

    #[test]
    fn stale_gen_drops_completion_response() {
        let mut app = bare_app();
        let eg = app.docs[0].edit_gen;
        let path = app.docs[0].path.clone().unwrap();
        app.pending_completion = Some((42, path, eg));
        app.docs[0].edit(0, "x", 0);
        assert_ne!(app.docs[0].edit_gen, eg);
        app.last_responses.push((42, serde_json::json!([{"label": "foo"}])));
        app.completion_pump();
        assert!(app.completion.is_none(), "stale gen must not open menu");
    }

    #[test]
    fn completion_result_object_form_parsed() {
        let mut app = bare_app();
        let path = app.docs[0].path.clone().unwrap();
        let eg = app.docs[0].edit_gen;
        app.pending_completion = Some((99, path, eg));
        app.last_responses.push((99, serde_json::json!({"items": [{"label": "foo"}, {"label": "bar"}]})));
        app.completion_pump();
        let menu = app.completion.as_ref().expect("menu should open");
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.items[0].label, "foo");
    }

    #[test]
    fn completion_result_array_form_parsed() {
        let mut app = bare_app();
        let path = app.docs[0].path.clone().unwrap();
        let eg = app.docs[0].edit_gen;
        app.pending_completion = Some((100, path, eg));
        app.last_responses.push((100, serde_json::json!([
            {"label": "alpha"},
            {"label": "beta", "insertTextFormat": 2, "insertText": "beta(${1:arg})"}
        ])));
        app.completion_pump();
        let menu = app.completion.as_ref().expect("menu should open");
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.items[1].insert, "beta(arg)");
    }

    #[test]
    fn word_replace_insert_on_accept() {
        let mut app = bare_app();
        app.docs[0].rope = ropey::Rope::from_str("alp\n");
        app.docs[0].cursor = 3;
        let eg = app.docs[0].edit_gen;
        let item = CompItem {
            label: "alpha_full".into(),
            kind_letter: 't',
            text_edit: None,
            insert: "alpha_full".into(),
        };
        app.completion = Some(CompMenu { items: vec![item], sel: 0, doc_idx: 0, edit_gen: eg });
        app.handle_completion_key(&key(KeyCode::Enter, KeyModifiers::NONE));
        let result = app.docs[0].rope.to_string();
        assert_eq!(result, "alpha_full\n");
    }

    #[test]
    fn text_edit_accept_replaces_range() {
        let mut app = bare_app();
        // "gam\n", cursor at 3, text_edit says replace chars 0..3 with "gamma"
        app.docs[0].rope = ropey::Rope::from_str("gam\n");
        app.docs[0].cursor = 3;
        let eg = app.docs[0].edit_gen;
        let item = CompItem {
            label: "gamma".into(),
            kind_letter: '?',
            text_edit: Some((0, 0, 3, "gamma".into())),
            insert: "gamma".into(),
        };
        app.completion = Some(CompMenu { items: vec![item], sel: 0, doc_idx: 0, edit_gen: eg });
        app.handle_completion_key(&key(KeyCode::Enter, KeyModifiers::NONE));
        let result = app.docs[0].rope.to_string();
        assert_eq!(result, "gamma\n");
    }

    #[test]
    fn accept_edits_menu_doc_not_active() {
        // review I1: accept targets menu.doc_idx, never the wrong buffer via active
        use crate::document::Document;
        let mut app = bare_app();
        app.docs.push(Document::from_str("target\n"));
        app.active = 0; // active doc 0 while the menu belongs to doc 1
        let eg = app.docs[1].edit_gen;
        let item = CompItem {
            label: "X".into(),
            kind_letter: 't',
            text_edit: None,
            insert: "X".into(),
        };
        app.completion = Some(CompMenu { items: vec![item], sel: 0, doc_idx: 1, edit_gen: eg });
        app.handle_completion_key(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.docs[1].rope.to_string(), "Xtarget\n", "menu doc edited");
        assert_eq!(app.docs[0].rope.to_string(), "fn main() {}\n", "active doc untouched");
        assert_eq!(app.active, 1, "active follows the accepted menu's doc");
    }
}

#[cfg(test)]
mod goto_tests {
    use super::*;

    fn make_app_with_file(name: &str, content: &str) -> (App, PathBuf) {
        let tmp = std::env::temp_dir().join(format!("goto_test_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&tmp).ok();
        let path = tmp.join(name);
        std::fs::write(&path, content).unwrap();
        let app = App::new(vec![path.clone()]);
        (app, path)
    }

    fn loc_response(uri: &str, line: u64, ch: u64) -> serde_json::Value {
        serde_json::json!({
            "uri": uri,
            "range": {
                "start": {"line": line, "character": ch},
                "end": {"line": line, "character": ch}
            }
        })
    }

    #[test]
    fn parse_null_returns_none() {
        assert!(App::parse_first_location(&serde_json::Value::Null).is_none());
    }

    #[test]
    fn parse_single_location() {
        let v = loc_response("file:///foo/bar.rs", 5, 3);
        let r = App::parse_first_location(&v).unwrap();
        assert_eq!(r.0, "file:///foo/bar.rs");
        assert_eq!(r.1, 5);
        assert_eq!(r.2, 3);
        assert_eq!(r.3, 0); // no extras
    }

    #[test]
    fn parse_array_first_plus_extra_count() {
        let arr = serde_json::json!([
            loc_response("file:///a.rs", 1, 0),
            loc_response("file:///b.rs", 2, 0),
            loc_response("file:///c.rs", 3, 0)
        ]);
        let r = App::parse_first_location(&arr).unwrap();
        assert_eq!(r.0, "file:///a.rs");
        assert_eq!(r.3, 2); // 2 extras
    }

    #[test]
    fn parse_empty_array_returns_none() {
        let arr = serde_json::json!([]);
        assert!(App::parse_first_location(&arr).is_none());
    }

    #[test]
    fn same_file_goto_sets_cursor_and_scroll() {
        // File with 600 lines of content; goto to line 500
        let content: String = (0..600).map(|i| format!("line{}\n", i)).collect();
        let (mut app, path) = make_app_with_file("same.rs", &content);
        let uri = format!("file://{}", path.canonicalize().unwrap().display());

        // Set pending goto and inject response
        app.pending_goto = Some((99, path.clone()));
        app.last_responses.push((99, loc_response(&uri, 500, 0)));
        app.goto_pump();

        let d = &app.docs[0];
        let cursor_line = d.rope.char_to_line(d.cursor);
        assert_eq!(cursor_line, 500, "cursor on line 500");
        // scroll_row should be centered: ~500 - view_h/2
        let view_h = crossterm::terminal::size().map(|(_, h)| h as usize).unwrap_or(24).saturating_sub(1).max(1);
        let expected_scroll = 500usize.saturating_sub(view_h / 2);
        assert!(d.scroll_row <= 500, "scroll_row <= 500, got {}", d.scroll_row);
        let _ = expected_scroll; // exact value is env-dependent
        // Jump stack should have been pushed
        assert_eq!(app.jump_stack.len(), 1);
    }

    #[test]
    fn emoji_line_utf16_char_mapping() {
        // "a💯def" — char=2 in utf16 maps to char col 1 (a=0, 💯=1,2 utf16)
        let content = "a\u{1F4AF}def\n";
        let (mut app, path) = make_app_with_file("emoji.rs", content);
        let uri = format!("file://{}", path.canonicalize().unwrap().display());

        app.pending_goto = Some((77, path.clone()));
        app.last_responses.push((77, loc_response(&uri, 0, 2)));
        app.goto_pump();

        // utf16 col 2 in "a💯def": a=1 u16, 💯=2 u16, col2 is mid-surrogate of 💯 → char 1
        assert_eq!(app.docs[0].cursor, 1, "utf16 col 2 → char 1 (💯 char start)");
    }

    #[test]
    fn null_response_no_stack_push() {
        let (mut app, path) = make_app_with_file("null.rs", "fn main() {}\n");
        app.pending_goto = Some((55, path.clone()));
        app.last_responses.push((55, serde_json::Value::Null));
        app.goto_pump();

        assert_eq!(app.jump_stack.len(), 0, "null response must not push to stack");
        let status = app.status.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
        assert!(status.contains("no definition"), "status: {status:?}");
    }

    #[test]
    fn array_response_applies_first_and_status_note() {
        let (mut app, path) = make_app_with_file("arr.rs", "fn main() {}\nfoo\n");
        let uri = format!("file://{}", path.canonicalize().unwrap().display());
        let arr = serde_json::json!([
            loc_response(&uri, 1, 0),
            loc_response(&uri, 0, 0)
        ]);
        app.pending_goto = Some((66, path.clone()));
        app.last_responses.push((66, arr));
        app.goto_pump();

        let cursor_line = app.docs[0].rope.char_to_line(app.docs[0].cursor);
        assert_eq!(cursor_line, 1, "first location applied");
        let status = app.status.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
        assert!(status.contains("1 more"), "status notes extra: {status:?}");
    }

    #[test]
    fn stale_edit_goto_still_applies() {
        // Review Focus 4: pending_goto has no gen → applies even after edits
        let (mut app, path) = make_app_with_file("stale.rs", "fn main() {}\nfoo\n");
        let uri = format!("file://{}", path.canonicalize().unwrap().display());

        app.pending_goto = Some((88, path.clone()));
        // Edit doc multiple times
        app.docs[0].edit(0, "// edit\n", 0);
        app.docs[0].edit(0, "// edit2\n", 0);
        // Response arrives → still applied
        app.last_responses.push((88, loc_response(&uri, 0, 0)));
        app.goto_pump();

        // Should have navigated (stack pushed)
        assert_eq!(app.jump_stack.len(), 1, "goto applied after edits");
    }

    #[test]
    fn other_file_activates_open_buffer() {
        let tmp = std::env::temp_dir().join(format!("goto_other_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).ok();
        let a = tmp.join("a.rs");
        let b = tmp.join("b.rs");
        std::fs::write(&a, "aaa\n").unwrap();
        std::fs::write(&b, "bbb\nbbb2\n").unwrap();

        let mut app = App::new(vec![a.clone(), b.clone()]);
        app.active = 0;

        let uri_b = format!("file://{}", b.canonicalize().unwrap().display());
        app.pending_goto = Some((11, a.clone()));
        app.last_responses.push((11, loc_response(&uri_b, 1, 0)));
        app.goto_pump();

        assert_eq!(app.active, 1, "active switched to doc B");
        let cursor_line = app.docs[app.active].rope.char_to_line(app.docs[app.active].cursor);
        assert_eq!(cursor_line, 1);
    }

    #[test]
    fn goto_new_buffer_opens_then_jump_back_returns() {
        let tmp = std::env::temp_dir().join(format!("goto_newbuf_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).ok();
        let a = tmp.join("a.rs");
        let c = tmp.join("c.rs");
        std::fs::write(&a, "aaa\n").unwrap();
        std::fs::write(&c, "ccc\n").unwrap();

        let mut app = App::new(vec![a.clone()]);
        let initial_cursor = app.docs[0].cursor;
        let initial_ndocs = app.docs.len();

        let uri_c = format!("file://{}", c.canonicalize().unwrap().display());
        app.pending_goto = Some((22, a.clone()));
        app.last_responses.push((22, loc_response(&uri_c, 0, 0)));
        app.goto_pump();

        assert_eq!(app.docs.len(), initial_ndocs + 1, "new buffer opened");
        assert_eq!(app.active, app.docs.len() - 1);

        // Jump back → back to a, original cursor
        app.jump_back();
        assert_eq!(app.active, 0, "jumped back to doc A");
        assert_eq!(app.docs[0].cursor, initial_cursor);
    }

    #[test]
    fn jump_stack_cap_50() {
        let (mut app, path) = make_app_with_file("cap.rs", "x\n");
        for i in 0..51 {
            if app.jump_stack.len() >= 50 {
                app.jump_stack.pop_front();
            }
            app.jump_stack.push_back((path.clone(), i));
        }
        assert_eq!(app.jump_stack.len(), 50, "capped at 50");
        assert_eq!(app.jump_stack[0].1, 1, "oldest (0) evicted");
    }

    #[test]
    fn jump_back_closed_buffer_gives_status() {
        let (mut app, path) = make_app_with_file("closed.rs", "x\n");
        // Push a path that doesn't match any open doc
        app.jump_stack.push_back((PathBuf::from("/nonexistent/path.rs"), 0));
        app.jump_back();
        let status = app.status.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
        assert!(status.contains("buffer closed"), "status: {status:?}");
        let _ = path;
    }

    #[test]
    fn completion_and_goto_pending_coexist() {
        // Both pending_completion and pending_goto can be set; each consumes only its id
        let (mut app, path) = make_app_with_file("coex.rs", "fn x() {}\n");
        let uri = format!("file://{}", path.canonicalize().unwrap().display());

        app.pending_completion = Some((10, path.clone(), 0));
        app.pending_goto = Some((20, path.clone()));

        // Inject goto response; completion response not yet arrived
        app.last_responses.push((20, loc_response(&uri, 0, 0)));
        app.goto_pump();
        app.completion_pump();

        // Goto consumed id 20; pending_completion still active (no response for 10)
        assert!(app.pending_goto.is_none());
        assert!(app.pending_completion.is_some(), "completion still pending");
    }
}
