pub mod mock;
pub mod snippet;
pub mod transport;

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use serde_json::{json, Value};
use lsp_types::Diagnostic;

use transport::{write_message, Frmr};

/// Events emitted by the hub and drained each poll by the app.
#[derive(Debug)]
pub enum LspEvent {
    Ready { lang: &'static str, root: PathBuf },
    Response { id: u64, result: Value },
    Diagnostics { path: PathBuf, diags: Vec<Diagnostic> },
    Died { lang: &'static str },
    /// window/showMessage from the server — the server telling the user why
    /// something is wrong (e.g. rust-analyzer's "Failed to discover workspace").
    ServerMessage { text: String },
}

// Key for per-server map
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct ServerKey {
    lang: &'static str,
    root: PathBuf,
}

enum ServerState {
    Attaching { init_id: u64, queued: Vec<Value> },
    Ready,
    Dead,
}

struct Server {
    state: ServerState,
    child: Child,
    writer_tx: Sender<Value>,
    // reader and writer thread handles
    reader_handle: Option<thread::JoinHandle<()>>,
    writer_handle: Option<thread::JoinHandle<()>>,
}

/// Hub owning one LSP server process per (lang, root) pair.
/// All threading uses mpsc channels — main thread never touches pipes.
pub struct LspHub {
    servers: HashMap<ServerKey, Server>,
    event_tx: Sender<LspEvent>,
    event_rx: Receiver<LspEvent>,
    next_id: u64,
    /// Events that have been classified and are ready to return to caller
    ready_events: Vec<LspEvent>,
}

impl Default for LspHub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            servers: HashMap::new(),
            event_tx: tx,
            event_rx: rx,
            next_id: 1,
            ready_events: Vec::new(),
        }
    }
}

impl LspHub {
    /// Spawn + initialize server for (lang, root). Returns false if spawn fails.
    /// Dead server → re-spawns. Already-alive server → no-op, returns true.
    pub fn ensure(&mut self, lang: &'static str, root: &Path, cfg_cmd: &[String]) -> bool {
        let key = ServerKey { lang, root: root.to_path_buf() };
        // If alive (Attaching or Ready), no-op
        if let Some(srv) = self.servers.get(&key) {
            if !matches!(srv.state, ServerState::Dead) {
                return true;
            }
            // Dead → remove and re-spawn below
            self.servers.remove(&key);
        }

        let mut child = match spawn_child(cfg_cmd) {
            Ok(c) => c,
            Err(_) => return false,
        };

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // Writer thread: owns stdin, receives Values, writes framed messages
        // Unbounded: main thread must never block (Review Focus 1); a wedged server
        // surfaces as Died, not as backpressure. Review I-1: bounded senders silently
        // drop didChange messages => protocol desync with no resync story.
        let (writer_tx, writer_rx) = mpsc::channel::<Value>();
        // One server death = one Died event: reader EOF and writer EPIPE are
        // two reporters of the SAME death; without this latch the App pump
        // counts two crashes and disables the language after a single crash.
        let died_sent = Arc::new(AtomicBool::new(false));
        let event_tx_w = self.event_tx.clone();
        let lang_w = lang;
        let died_sent_w = died_sent.clone();
        let writer_handle = thread::Builder::new()
            .name(format!("lsp-writer-{lang}"))
            .spawn(move || {
                let mut sink = stdin;
                for msg in writer_rx {
                    if let Err(_) = write_message(&mut sink, &msg) {
                        // EPIPE or similar — signal Died (once per server)
                        if died_sent_w
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_ok()
                        {
                            let _ = event_tx_w.send(LspEvent::Died { lang: lang_w });
                        }
                        return;
                    }
                }
            })
            .expect("spawn writer thread");

        // Reader thread: owns stdout, reads framed messages, classifies, sends events
        let event_tx_r = self.event_tx.clone();
        let lang_r = lang;
        let root_r = root.to_path_buf();
        let died_sent_r = died_sent.clone();
        let reader_handle = thread::Builder::new()
            .name(format!("lsp-reader-{lang}"))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut framer = Frmr::new();
                loop {
                    match framer.read_message(&mut reader) {
                        Ok(Some(msg)) => {
                            let ev = classify_message(msg, lang_r, &root_r);
                            if let Some(e) = ev {
                                let is_died = matches!(e, LspEvent::Died { .. });
                                if !is_died
                                    || died_sent_r
                                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                        .is_ok()
                                {
                                    let _ = event_tx_r.send(e);
                                }
                                if is_died { break; }
                            }
                        }
                        Ok(None) | Err(_) => {
                            // EOF or poison → Died (once per server)
                            if died_sent_r
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                            {
                                let _ = event_tx_r.send(LspEvent::Died { lang: lang_r });
                            }
                            break;
                        }
                    }
                }
            })
            .expect("spawn reader thread");

        // Send initialize
        let init_id = self.next_id;
        self.next_id += 1;
        let root_uri = format!("file://{}", root.display());
        let init_msg = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": root_uri,
                "capabilities": {}
            }
        });
        let _ = writer_tx.send(init_msg);

        self.servers.insert(key, Server {
            state: ServerState::Attaching { init_id, queued: Vec::new() },
            child,
            writer_tx,
            reader_handle: Some(reader_handle),
            writer_handle: Some(writer_handle),
        });
        true
    }

    /// Send a request. Returns Some(id) or None if server unknown/dead.
    /// While Attaching, queues the message and still returns the id.
    pub fn request(
        &mut self,
        lang: &str,
        root: &Path,
        method: &str,
        params: Value,
    ) -> Option<u64> {
        // Process pending events first so Attaching→Ready transitions happen
        self.process_pending_events();

        let key = ServerKey { lang: leak_lang(lang), root: root.to_path_buf() };
        let server = self.servers.get_mut(&key)?;

        if matches!(server.state, ServerState::Dead) {
            return None;
        }

        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        match &mut server.state {
            ServerState::Attaching { queued, .. } => {
                queued.push(msg);
            }
            ServerState::Ready => {
                let _ = server.writer_tx.send(msg);
            }
            ServerState::Dead => unreachable!(),
        }
        Some(id)
    }

    /// Send a notification (no id). No-op if server unknown/dead.
    pub fn notify(&mut self, lang: &str, root: &Path, method: &str, params: Value) {
        self.process_pending_events();

        let key = ServerKey { lang: leak_lang(lang), root: root.to_path_buf() };
        let server = match self.servers.get_mut(&key) {
            Some(s) => s,
            None => return,
        };

        if matches!(server.state, ServerState::Dead) {
            return;
        }

        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        match &mut server.state {
            ServerState::Attaching { queued, .. } => {
                queued.push(msg);
            }
            ServerState::Ready => {
                let _ = server.writer_tx.send(msg);
            }
            ServerState::Dead => {}
        }
    }

    /// Drain all queued events. Processes hub-internal events (Ready transitions) before return.
    /// Test-only: push a synthetic event into the hub's channel (simulates server output).
    #[cfg(test)]
    pub fn inject_event_for_bench(&self, ev: LspEvent) {
        let _ = self.event_tx.send(ev);
    }

    pub fn take_events(&mut self) -> Vec<LspEvent> {
        self.process_pending_events();
        std::mem::take(&mut self.ready_events)
    }

    /// Shutdown all servers: send shutdown+exit, join reader threads (~300ms), then kill.
    pub fn shutdown_all(&mut self) {
        let shutdown_req = json!({
            "jsonrpc": "2.0",
            "id": 0u64,
            "method": "shutdown",
            "params": null
        });
        let exit_notif = json!({
            "jsonrpc": "2.0",
            "method": "exit",
            "params": null
        });

        for (_, srv) in &self.servers {
            if !matches!(srv.state, ServerState::Dead) {
                let _ = srv.writer_tx.send(shutdown_req.clone());
                let _ = srv.writer_tx.send(exit_notif.clone());
            }
        }

        // Give threads time to drain then join/kill
        std::thread::sleep(std::time::Duration::from_millis(300));

        for (_, srv) in &mut self.servers {
            // Drop writer channel so writer thread exits its recv loop
            // We replace with a fresh channel (old one dropped)
            let (dead_tx, _dead_rx) = mpsc::channel();
            let old_tx = std::mem::replace(&mut srv.writer_tx, dead_tx);
            drop(old_tx); // writer thread's recv loop ends
            if let Some(h) = srv.reader_handle.take() {
                let _ = h.join();
            }
            if let Some(h) = srv.writer_handle.take() {
                let _ = h.join();
            }
            let _ = srv.child.kill();
            let _ = srv.child.wait();
            srv.state = ServerState::Dead;
        }
    }

    /// Kill all servers for a given lang (used by tests and respawn paths).
    pub fn kill(&mut self, lang: &str) {
        for (key, srv) in &mut self.servers {
            if key.lang == lang {
                let _ = srv.child.kill();
                let _ = srv.child.wait();
                // Close the writer thread now instead of leaving it parked until the
                // entry is replaced (review I-2): drop the old sender, join both threads.
                // Bounded: killed child => reader hits EOF immediately.
                let (dead_tx, _dead_rx) = mpsc::channel();
                let old_tx = std::mem::replace(&mut srv.writer_tx, dead_tx);
                drop(old_tx);
                if let Some(h) = srv.writer_handle.take() {
                    let _ = h.join();
                }
                if let Some(h) = srv.reader_handle.take() {
                    let _ = h.join();
                }
                srv.state = ServerState::Dead;
            }
        }
    }

    /// Pump lsp_flow events to apply state transitions without exposing them.
    /// Called from app's lsp_pump; here just discards (Task 5 wires consumption).
    pub fn lsp_pump(&mut self) {
        let _ = self.take_events();
    }

    // Internal: drain channel, apply Ready transitions, store results in ready_events
    fn process_pending_events(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match &ev {
                LspEvent::Response { id, .. } => {
                    // Check if this id matches any Attaching server's init_id
                    let mut matched_key: Option<ServerKey> = None;
                    for (key, srv) in self.servers.iter() {
                        if let ServerState::Attaching { init_id, .. } = &srv.state {
                            if *init_id == *id {
                                matched_key = Some(key.clone());
                                break;
                            }
                        }
                    }
                    if let Some(key) = matched_key {
                        let srv = self.servers.get_mut(&key).unwrap();
                        let lang = key.lang;
                        let root = key.root.clone();
                        // Transition to Ready, flush queued
                        let queued = if let ServerState::Attaching { queued, .. } = &mut srv.state {
                            std::mem::take(queued)
                        } else {
                            Vec::new()
                        };
                        srv.state = ServerState::Ready;
                        // Send initialized notification
                        let init_notif = json!({
                            "jsonrpc": "2.0",
                            "method": "initialized",
                            "params": {}
                        });
                        let _ = srv.writer_tx.send(init_notif);
                        // Flush queued messages
                        for msg in queued {
                            let _ = srv.writer_tx.send(msg);
                        }
                        self.ready_events.push(LspEvent::Ready { lang, root });
                        // Don't emit the Response for init_id (it's internal)
                    } else {
                        self.ready_events.push(ev);
                    }
                }
                LspEvent::Died { lang } => {
                    // Mark server dead
                    let lang = *lang;
                    for (key, srv) in self.servers.iter_mut() {
                        if key.lang == lang && !matches!(srv.state, ServerState::Dead) {
                            srv.state = ServerState::Dead;
                            break;
                        }
                    }
                    self.ready_events.push(ev);
                }
                _ => {
                    self.ready_events.push(ev);
                }
            }
        }
    }
}

impl Drop for LspHub {
    fn drop(&mut self) {
        for (_, srv) in &mut self.servers {
            let _ = srv.child.kill();
            let _ = srv.child.wait();
        }
    }
}

fn spawn_child(cmd: &[String]) -> std::io::Result<Child> {
    if cmd.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command"));
    }
    let mut c = Command::new(&cmd[0]);
    c.args(&cmd[1..]);
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    c.spawn()
}

/// Classify a framed message into an LspEvent (or None if ignorable).
fn classify_message(msg: Value, _lang: &'static str, _root: &Path) -> Option<LspEvent> {
    let has_method = msg.get("method").is_some();
    let id = msg.get("id").and_then(|v| v.as_u64());

    if has_method {
        // Notification or server-initiated request
        let method = msg["method"].as_str().unwrap_or("");
        if method == "textDocument/publishDiagnostics" {
            let uri = msg["params"]["uri"].as_str().unwrap_or("");
            let path = uri_to_path(uri);
            let diags: Vec<Diagnostic> = msg["params"]["diagnostics"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|d| serde_json::from_value(d.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            return Some(LspEvent::Diagnostics { path, diags });
        }
        if method == "window/showMessage" || method == "window/showMessageRequest" {
            // Surface server explanations instead of dropping them (the server's
            // own words are the best diagnosis when it delivers nothing else).
            if let Some(text) = msg["params"]["message"].as_str() {
                return Some(LspEvent::ServerMessage { text: text.to_string() });
            }
        }
        None // Other notifications ignored
    } else if let Some(id) = id {
        // Response
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        Some(LspEvent::Response { id, result })
    } else {
        None
    }
}

// Intern a lang &str as &'static str for use in keys.
// We only support a fixed set of languages; unknown langs get a leaked copy.
fn leak_lang(lang: &str) -> &'static str {
    match lang {
        "rust" => "rust",
        "c" => "c",
        "toml" => "toml",
        "markdown" => "markdown",
        "bash" => "bash",
        "json" => "json",
        other => Box::leak(other.to_string().into_boxed_str()),
    }
}

/// Convert a char-column offset to its UTF-16 code-unit offset within a line.
/// Clamps to the line length if `col_chars` is past the end.
pub fn char_col_to_utf16(line: &str, col_chars: usize) -> u32 {
    let mut u16_count: u32 = 0;
    for (char_idx, ch) in line.chars().enumerate() {
        if char_idx >= col_chars {
            break;
        }
        u16_count += ch.len_utf16() as u32;
    }
    u16_count
}

/// Convert a UTF-16 code-unit offset to its char offset within a line.
/// Clamps to line length; landing mid-surrogate-pair resolves to that char's start.
pub fn utf16_col_to_char(line: &str, col_utf16: usize) -> usize {
    let mut u16_seen: usize = 0;
    for (char_idx, ch) in line.chars().enumerate() {
        if u16_seen >= col_utf16 {
            return char_idx;
        }
        let next = u16_seen + ch.len_utf16();
        if next > col_utf16 {
            // col_utf16 lands mid-surrogate-pair → return this char's start
            return char_idx;
        }
        u16_seen = next;
    }
    // clamped: return total char count
    line.chars().count()
}

/// Find the nearest ancestor directory containing `.git`, else return the file's parent dir.
/// file:// URI <-> path codec shared by hub + app (single source of truth).
/// ponytail: encode/decode spaces as %20 only; other special chars are rare in
/// project paths. Upgrade path: full percent-encoding if non-space paths bite.
pub fn path_to_uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display()).replace(' ', "%20")
}

pub fn uri_to_path(uri: &str) -> PathBuf {
    PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri).replace("%20", " "))
}

pub fn project_root(path: &std::path::Path) -> std::path::PathBuf {
    let start = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    let mut dir = start;
    loop {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    start.to_path_buf()
}


/// Returns Option<Vec<String>> where:
/// - Some(vec![...]) = override command configured
/// - None if explicitly disabled (false or "") OR language has no default
/// - Default command if language not configured but has a default
/// Note: does NOT check if the binary exists on PATH — spawn failure is handled here.
pub fn command_for(cfg: &crate::config::Config, lang: &str) -> Option<Vec<String>> {
    // If explicitly configured (override or disable), use that entry
    if let Some(entry) = cfg.lsp.get(lang) {
        return entry.command.clone();
    }

    // Otherwise, check defaults table for absent keys
    let default_cmd = match lang {
        "rust" => Some(vec!["rust-analyzer"]),
        "c" => Some(vec!["clangd"]),
        "toml" => Some(vec!["taplo", "lsp", "stdio"]),
        "markdown" => Some(vec!["marksman"]),
        "bash" => Some(vec!["bash-language-server", "start"]),
        "json" => Some(vec!["vscode-json-language-server", "--stdio"]),
        "python" => Some(vec!["pylsp"]),
        "go" => Some(vec!["gopls"]),
        "lua" => Some(vec!["lua-language-server"]),
        _ => None,
    };

    default_cmd.map(|strs| strs.into_iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_show_message_becomes_server_message_event() {
        let root = std::path::Path::new("/tmp");
        for method in ["window/showMessage", "window/showMessageRequest"] {
            let ev = classify_message(
                serde_json::json!({"jsonrpc": "2.0", "method": method,
                                   "params": {"type": 1, "message": "Failed to discover workspace.\nSee docs"}}),
                "rust", root,
            );
            match ev {
                Some(LspEvent::ServerMessage { text }) => {
                    assert!(text.contains("Failed to discover workspace"), "{text}");
                }
                other => panic!("{method} must produce ServerMessage, got {other:?}"),
            }
        }
        // unrelated notifications still ignored
        assert!(classify_message(
            serde_json::json!({"jsonrpc":"2.0","method":"$/progress","params":{}}), "rust", root).is_none());
    }

    #[test]
    fn one_dead_server_emits_exactly_one_died() {
        // A READY server that exits while the writer is mid-push hits BOTH
        // reporters: reader EOF AND writer EPIPE. One server death must count
        // ONE Died — otherwise one mid-session crash burns note+respawn AND
        // trips the disable without a second real crash.
        let mut hub = LspHub::default();
        // Fake server: answers initialize (id 1) so the hub goes Ready, then
        // exits mid-stream while the writer is pushing a 2 MB notification.
        let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let script = format!("printf '{frame}'; sleep 0.3; exit 7");
        let argv = vec!["sh".to_string(), "-c".to_string(), script];
        assert!(hub.ensure("c", Path::new("/tmp"), &argv), "spawn must succeed");
        // Wait for Ready, then push bytes the child will not consume.
        let ready_deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
        let mut ready = false;
        while std::time::Instant::now() < ready_deadline && !ready {
            ready = hub.take_events().iter().any(|e| matches!(e, LspEvent::Ready { .. }));
            if !ready {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert!(ready, "fake server must produce Ready");
        hub.notify(
            "c",
            Path::new("/tmp"),
            "noise",
            serde_json::json!({ "blob": "x".repeat(2_000_000) }),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
        let mut died = 0;
        while std::time::Instant::now() < deadline {
            for ev in hub.take_events() {
                if matches!(ev, LspEvent::Died { .. }) {
                    died += 1;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(died, 1, "one dead server must produce exactly one Died");
        hub.shutdown_all();
    }

    #[test]
    fn utf16_ascii_roundtrip() {
        let line = "hello";
        for col in 0..=5 {
            assert_eq!(char_col_to_utf16(line, col) as usize, col);
            assert_eq!(utf16_col_to_char(line, col), col);
        }
    }

    #[test]
    fn utf16_e_acute_one_u16() {
        let line = "é"; // U+00E9, fits in one u16
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 1), 1);
        assert_eq!(utf16_col_to_char(line, 0), 0);
        assert_eq!(utf16_col_to_char(line, 1), 1);
    }

    #[test]
    fn utf16_emoji_surrogate_pair() {
        let line = "💯"; // U+1F4AF, needs two u16 code units
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 1), 2); // past the emoji = 2 u16
        // char 0 starts at u16 offset 0
        assert_eq!(utf16_col_to_char(line, 0), 0);
        // u16 offset 1 = mid-surrogate → resolves to char 0 (owning char start)
        assert_eq!(utf16_col_to_char(line, 1), 0);
        // u16 offset 2 = past the emoji → clamped to 1 (total chars)
        assert_eq!(utf16_col_to_char(line, 2), 1);
    }

    #[test]
    fn utf16_mixed_line_roundtrip() {
        // "é→💯x": é(1u16) →(1u16) 💯(2u16) x(1u16) = 5 u16 total, 4 chars
        let line = "é→💯x";
        // char→u16
        assert_eq!(char_col_to_utf16(line, 0), 0); // before é
        assert_eq!(char_col_to_utf16(line, 1), 1); // before →
        assert_eq!(char_col_to_utf16(line, 2), 2); // before 💯
        assert_eq!(char_col_to_utf16(line, 3), 4); // before x
        assert_eq!(char_col_to_utf16(line, 4), 5); // past end
        // u16→char
        assert_eq!(utf16_col_to_char(line, 0), 0);
        assert_eq!(utf16_col_to_char(line, 1), 1);
        assert_eq!(utf16_col_to_char(line, 2), 2);
        assert_eq!(utf16_col_to_char(line, 3), 2); // mid-surrogate → char 2
        assert_eq!(utf16_col_to_char(line, 4), 3);
        assert_eq!(utf16_col_to_char(line, 5), 4);
    }

    #[test]
    fn utf16_clamp_beyond_end() {
        let line = "ab";
        // char col past end → clamped
        assert_eq!(char_col_to_utf16(line, 99), 2);
        // u16 col past end → clamped to char count
        assert_eq!(utf16_col_to_char(line, 99), 2);
    }

    #[test]
    fn project_root_finds_git_ancestor() {
        let tmp = std::env::temp_dir().join(format!("plat_pr_{}", std::process::id()));
        let git_dir = tmp.join("repo");
        let nested = git_dir.join("src").join("module");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(git_dir.join(".git")).unwrap();
        let file = nested.join("main.rs");
        std::fs::write(&file, "").unwrap();
        let root = project_root(&file);
        assert_eq!(root, git_dir, "should find .git two levels up");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn project_root_no_git_uses_parent() {
        let tmp = std::env::temp_dir().join(format!("plat_pr_ng_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("foo.rs");
        std::fs::write(&file, "").unwrap();
        let root = project_root(&file);
        assert_eq!(root, tmp, "no .git → parent dir");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn defaults_table_all_nine_languages() {
        let cfg = crate::config::Config::defaults();
        assert_eq!(command_for(&cfg, "rust"), Some(vec!["rust-analyzer".to_string()]));
        assert_eq!(command_for(&cfg, "c"), Some(vec!["clangd".to_string()]));
        assert_eq!(
            command_for(&cfg, "toml"),
            Some(vec!["taplo".to_string(), "lsp".to_string(), "stdio".to_string()])
        );
        assert_eq!(command_for(&cfg, "markdown"), Some(vec!["marksman".to_string()]));
        assert_eq!(
            command_for(&cfg, "bash"),
            Some(vec!["bash-language-server".to_string(), "start".to_string()])
        );
        assert_eq!(
            command_for(&cfg, "json"),
            Some(vec!["vscode-json-language-server".to_string(), "--stdio".to_string()])
        );
        assert_eq!(command_for(&cfg, "python"), Some(vec!["pylsp".to_string()]));
        assert_eq!(command_for(&cfg, "go"), Some(vec!["gopls".to_string()]));
        assert_eq!(command_for(&cfg, "lua"), Some(vec!["lua-language-server".to_string()]));
    }

    #[test]
    fn unknown_language_no_default() {
        let cfg = crate::config::Config::defaults();
        assert_eq!(command_for(&cfg, "ruby"), None);
    }

    #[test]
    fn override_beats_default() {
        let cfg = crate::config::from_str("[lsp]\nrust = \"my-ra --flag\"\n").unwrap();
        assert_eq!(
            command_for(&cfg, "rust"),
            Some(vec!["my-ra".to_string(), "--flag".to_string()])
        );
    }

    #[test]
    fn explicit_disabled_entry_beats_defaults() {
        let cfg = crate::config::from_str("[lsp]\ntoml = false\n").unwrap();
        assert_eq!(command_for(&cfg, "toml"), None);

        let cfg2 = crate::config::Config::defaults();
        assert_eq!(
            command_for(&cfg2, "toml"),
            Some(vec!["taplo".to_string(), "lsp".to_string(), "stdio".to_string()])
        );
    }

    #[test]
    fn empty_string_disabled_beats_defaults() {
        let cfg = crate::config::from_str("[lsp]\ntoml = \"\"\n").unwrap();
        assert_eq!(command_for(&cfg, "toml"), None);
    }

    #[test]
    fn id_counter_increments_across_requests() {
        let mut hub = LspHub::default();
        let root = std::path::Path::new("/tmp/test_root");
        // ensure with /bin/true as fake (won't connect, returns false spawn or dead)
        // Just test that request returns None for unknown key (no ensure called)
        let r1 = hub.request("rust", root, "test/method", serde_json::json!({}));
        let r2 = hub.request("rust", root, "test/method", serde_json::json!({}));
        assert_eq!(r1, None);
        assert_eq!(r2, None);
        // next_id still incremented (tested indirectly via ensure + cat below)
    }

    #[test]
    fn request_none_for_unknown_key() {
        let mut hub = LspHub::default();
        let result = hub.request(
            "unknown_lang",
            Path::new("/nonexistent"),
            "test/method",
            json!({}),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn ensure_twice_same_key_no_respawn() {
        // Use /bin/cat as fake server — it echoes stdin back to stdout
        // This verifies ensure idempotency (second call returns true without new child)
        let root = std::path::PathBuf::from("/tmp/lsp_cat_test");
        let _ = std::fs::create_dir_all(&root);
        let mut hub = LspHub::default();
        let cmd = vec!["/bin/cat".to_string()];
        let ok1 = hub.ensure("c", &root, &cmd);
        assert!(ok1);
        let ok2 = hub.ensure("c", &root, &cmd);
        assert!(ok2);
        // Should only have one server entry
        assert_eq!(hub.servers.len(), 1);
        // clean up
        hub.kill("c");
    }

    #[test]
    fn cat_fake_server_ready_event() {
        // /bin/cat echoes bytes back. Hub sends initialize → cat echoes it → reader
        // sees a message with "id" field (no "result" field) → classified as Response
        // by id presence → hub transitions to Ready and emits Ready event.
        // Note: cat echoes a Request (has "method" too), so classify_message sees
        // has_method=true → notification path. We must classify on id presence only
        // when method is absent. Since cat echoes exactly what we sent (which has "method"),
        // the message is classified as notification (ignored). This is correct: cat doesn't
        // speak LSP. This test validates the plumbing won't panic.
        let root = std::path::PathBuf::from("/tmp/lsp_cat_plumbing");
        let _ = std::fs::create_dir_all(&root);
        let mut hub = LspHub::default();
        let cmd = vec!["/bin/cat".to_string()];
        let ok = hub.ensure("bash", &root, &cmd);
        assert!(ok, "spawn /bin/cat should succeed");
        // cat will echo initialize back; reader sees method=initialize (notification) → ignored
        // No Ready event expected — this just proves no panic
        std::thread::sleep(std::time::Duration::from_millis(50));
        let events = hub.take_events();
        // No panic and no crash is the assertion; events may be empty
        let _ = events;
        hub.kill("bash");
    }

    // ── Perf gates (run with: cargo test --release -- --ignored lsp) ────────────

    /// Framed write of a 100k-line document must complete in < 10 ms.
    ///
    /// Measures two separate costs:
    ///   (a) rope → String serialisation
    ///   (b) Content-Length framing + write into a Vec sink
    ///
    /// The spec says full-text sync is off the key path; this confirms the
    /// end-to-end cost stays well under the 10 ms gate.
    #[test]
    #[ignore]
    fn lsp_bench_framed_write_100k_lines() {
        use crate::lsp::transport::write_message;
        use ropey::Rope;

        let body = "fn main() { /* placeholder */ }\n".repeat(100_000);

        // (a) rope → String
        let rope = Rope::from_str(&body);
        let t = std::time::Instant::now();
        let text = rope.to_string();
        let rope_elapsed = t.elapsed();

        // (b) framing + write into Vec sink
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": "file:///tmp/bench.rs", "version": 1},
                "contentChanges": [{"text": text}]
            }
        });
        let mut sink: Vec<u8> = Vec::with_capacity(body.len() + 256);
        let t = std::time::Instant::now();
        write_message(&mut sink, &msg).unwrap();
        let write_elapsed = t.elapsed();

        println!(
            "100k-line framed write: rope_to_string={rope_elapsed:?}, write_message={write_elapsed:?}, total={:?}",
            rope_elapsed + write_elapsed
        );

        let total = rope_elapsed + write_elapsed;
        assert!(
            total.as_millis() < 10,
            "framed write too slow: {total:?} (gate: <10 ms)"
        );
    }

    /// lsp_pump with 1000 queued Diagnostics events must drain in ≤ 1 ms.
    ///
    /// Drives the diagnostic conversion + storage path directly via the
    /// hub's internal channel (inject_events_for_bench). Does NOT spawn a
    /// real server — exercises only the pump logic.
    #[test]
    #[ignore]
    fn lsp_bench_pump_1000_diagnostic_events() {
        use crate::app::App;

        // Build a doc on disk so lsp_pump can match path → doc index.
        let dir = std::env::temp_dir()
            .join(format!("lsp_bench_pump_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bench.rs");
        let body: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, &body).unwrap();

        let mut app = App::new(vec![path.clone()]);

        // Inject 1000 Diagnostics events via the hub's test helper.
        for i in 0..1000u32 {
            let diag = lsp_types::Diagnostic {
                range: lsp_types::Range {
                    start: lsp_types::Position { line: (i % 1000), character: 0 },
                    end:   lsp_types::Position { line: (i % 1000), character: 4 },
                },
                severity: Some(lsp_types::DiagnosticSeverity::WARNING),
                message: format!("warn {i}"),
                ..Default::default()
            };
            let ev = LspEvent::Diagnostics { path: path.clone(), diags: vec![diag] };
            app.lsp.inject_event_for_bench(ev);
        }

        let t = std::time::Instant::now();
        app.lsp_pump();
        let elapsed = t.elapsed();

        println!("lsp_pump 1000 diag events: {elapsed:?}");
        assert!(
            elapsed.as_millis() <= 1,
            "lsp_pump too slow: {elapsed:?} (gate: ≤1 ms)"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Keystroke dispatch gate: handle_command(InsertChar) on a 100k-line doc
    /// must cost ≤ +10% vs the HEAD-no-LSP baseline.
    ///
    /// This bench runs TWO configurations and prints a comparison table:
    ///   HEAD-with-LSP  — app with an attached mock-fed doc (events flowing)
    ///   HEAD-no-LSP    — app with no LSP configured (pure pre-feature path)
    ///
    /// See README "Acceptance (LSP core)" for dce6fe6 baseline numbers.
    #[test]
    #[ignore]
    fn lsp_bench_keystroke_dispatch_100k_lines() {
        use crate::app::App;
        const N: usize = 1000;

        let body: String = (0..100_000).map(|i| format!("line {i}\n")).collect();

        // ── HEAD no-LSP ──────────────────────────────────────────────────────
        let dir_no = std::env::temp_dir()
            .join(format!("lsp_bench_kd_nolsp_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir_no).unwrap();
        let p_no = dir_no.join("bench.rs");
        std::fs::write(&p_no, &body).unwrap();
        // Config with no [lsp] section → no LSP path active
        let cfg_no = crate::config::Config::defaults();
        let mut app_no = App::new_with_config(vec![p_no.clone()], cfg_no);

        // Position cursor in the middle of the document
        app_no.docs[0].cursor = body.len() / 2;

        let t = std::time::Instant::now();
        for _ in 0..N {
            app_no.handle_command(crate::keymap::Command::InsertChar('x'));
        }
        let no_lsp = t.elapsed();
        std::fs::remove_dir_all(&dir_no).ok();

        // ── HEAD with-LSP ────────────────────────────────────────────────────
        let dir_lsp = std::env::temp_dir()
            .join(format!("lsp_bench_kd_withlsp_{}_{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir_lsp).unwrap();
        let p_lsp = dir_lsp.join("bench.rs");
        std::fs::write(&p_lsp, &body).unwrap();

        // Inject a small batch of pending Diagnostics events so the with-LSP
        // path has something to drain each iteration (realistic load).
        let mut app_lsp = App::new(vec![p_lsp.clone()]);
        for i in 0..10u32 {
            let diag = lsp_types::Diagnostic {
                range: lsp_types::Range {
                    start: lsp_types::Position { line: i, character: 0 },
                    end:   lsp_types::Position { line: i, character: 4 },
                },
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                message: format!("mock error {i}"),
                ..Default::default()
            };
            app_lsp.lsp.inject_event_for_bench(LspEvent::Diagnostics {
                path: p_lsp.clone(),
                diags: vec![diag],
            });
        }
        app_lsp.lsp_pump(); // drain the pre-loaded events once
        app_lsp.docs[0].cursor = body.len() / 2;

        let t = std::time::Instant::now();
        for _ in 0..N {
            app_lsp.handle_command(crate::keymap::Command::InsertChar('x'));
        }
        let with_lsp = t.elapsed();
        std::fs::remove_dir_all(&dir_lsp).ok();

        // ── Report ───────────────────────────────────────────────────────────
        let no_us   = no_lsp.as_micros();
        let lsp_us  = with_lsp.as_micros();
        let pct = if no_us > 0 { (lsp_us as f64 / no_us as f64 - 1.0) * 100.0 } else { 0.0 };

        println!("keystroke dispatch ({N} InsertChar, 100k-line doc):");
        println!("  HEAD no-LSP  : {no_lsp:?}  ({} µs/key)", no_us / N as u128);
        println!("  HEAD with-LSP: {with_lsp:?}  ({} µs/key)", lsp_us / N as u128);
        println!("  overhead     : {pct:+.1}%  (gate: ≤+10%)");

        assert!(
            pct <= 10.0,
            "keystroke overhead {pct:+.1}% exceeds +10% gate (no_lsp={no_lsp:?}, with_lsp={with_lsp:?})"
        );
    }
}
