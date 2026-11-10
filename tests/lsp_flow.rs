/// Integration tests for LspHub + PLAT_LSP_MOCK dev server (Task 3).
/// Mock is spawned via: ["sh", "-c", "PLAT_LSP_MOCK=1 <bin>"]
/// Hub never sets env vars — the sh -c form embeds it.

use plat::lsp::{LspEvent, LspHub};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PLAT_BIN: &str = env!("CARGO_BIN_EXE_plat");

fn mock_cmd() -> Vec<String> {
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("PLAT_LSP_MOCK=1 {}", PLAT_BIN),
    ]
}

fn wait_event<F: Fn(&LspEvent) -> bool>(
    hub: &mut LspHub,
    pred: F,
    timeout: Duration,
) -> Option<LspEvent> {
    let deadline = Instant::now() + timeout;
    loop {
        for ev in hub.take_events() {
            if pred(&ev) {
                return Some(ev);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn temp_root() -> PathBuf {
    // Unique dir per call: the 19 tests run as parallel threads in ONE
    // process and used to share lsp_test_<pid>/main.rs — one test's shorter
    // file content racing another test's open flipped which line LSP
    // diagnostics clamped to (flake hunt 2026-11-05: diag_ux_end_to_end).
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lsp_test_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Test 1: ensure(mock) → Ready event
#[test]
fn test_ensure_ready() {
    let root = temp_root();
    let mut hub = LspHub::default();
    let ok = hub.ensure("rust", &root, &mock_cmd());
    assert!(ok, "ensure should return true");

    let ev = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Ready { .. }),
        Duration::from_millis(500),
    );
    assert!(ev.is_some(), "expected Ready event within 500ms");
    hub.shutdown_all();
}

// Test 2: didOpen → Diagnostics with 2 diags
#[test]
fn test_did_open_diagnostics() {
    let root = temp_root();
    let doc_path = root.join("main.rs");
    std::fs::write(&doc_path, "fn main() {}\nlet x = 1;\nlet y = 2;\n").unwrap();
    let doc_uri = format!("file://{}", doc_path.display());

    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Ready { .. }),
        Duration::from_millis(500),
    )
    .expect("Ready");

    hub.notify(
        "rust",
        &root,
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": "fn main() {}\nlet x = 1;\nlet y = 2;\n"
            }
        }),
    );

    let ev = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Diagnostics { .. }),
        Duration::from_millis(500),
    )
    .expect("Diagnostics event");

    if let LspEvent::Diagnostics { path: _, diags } = ev {
        assert_eq!(diags.len(), 2, "expected 2 diagnostics");
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(msgs.contains(&"mock error"), "missing mock error, got: {:?}", msgs);
        assert!(msgs.contains(&"mock warning"), "missing mock warning, got: {:?}", msgs);
        // lines are 0-based
        let lines: Vec<u32> = diags
            .iter()
            .map(|d| d.range.start.line)
            .collect();
        assert!(lines.contains(&0), "error should be on line 0 (1 in human), got: {:?}", lines);
        assert!(lines.contains(&1), "warning should be on line 1 (2 in human), got: {:?}", lines);
    }
    hub.shutdown_all();
}

// Test 3: completion request → Response with 3 items
#[test]
fn test_completion_response() {
    let root = temp_root();
    let doc_path = root.join("comp.rs");
    std::fs::write(&doc_path, "fn main() {}\nlet x = 1;\nlet y = 2;\n").unwrap();
    let doc_uri = format!("file://{}", doc_path.display());

    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready");

    // didOpen first so mock has state
    hub.notify(
        "rust",
        &root,
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": &doc_uri, "languageId": "rust", "version": 1,
                "text": "fn main() {}\n"
            }
        }),
    );
    wait_event(&mut hub, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(500))
        .expect("Diagnostics after didOpen");

    let id = hub
        .request(
            "rust",
            &root,
            "textDocument/completion",
            serde_json::json!({
                "textDocument": {"uri": &doc_uri},
                "position": {"line": 0, "character": 5}
            }),
        )
        .expect("request returns Some(id)");

    let ev = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Response { id: eid, .. } if *eid == id),
        Duration::from_millis(500),
    )
    .expect("Response event");

    if let LspEvent::Response { result, .. } = ev {
        // Mock returns array form
        let items = result.as_array().expect("result is array");
        assert_eq!(items.len(), 3, "expected 3 completion items");
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or(""))
            .collect();
        assert!(labels.contains(&"alpha_full"), "missing alpha_full, got: {:?}", labels);
        assert!(
            labels.iter().any(|l| l.contains("beta")),
            "missing beta item, got: {:?}",
            labels
        );
        assert!(labels.contains(&"gamma"), "missing gamma, got: {:?}", labels);
    }
    hub.shutdown_all();
}

// Test 4: definition request → Response → Location uri matches doc
#[test]
fn test_definition_response() {
    let root = temp_root();
    let doc_path = root.join("def.rs");
    std::fs::write(&doc_path, "fn foo() {}\n").unwrap();
    let doc_uri = format!("file://{}", doc_path.display());

    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready");

    let id = hub
        .request(
            "rust",
            &root,
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": &doc_uri},
                "position": {"line": 0, "character": 0}
            }),
        )
        .expect("Some(id)");

    let ev = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Response { id: eid, .. } if *eid == id),
        Duration::from_millis(500),
    )
    .expect("definition Response");

    if let LspEvent::Response { result, .. } = ev {
        // Mock returns single Location object
        let uri = result["uri"].as_str().expect("has uri");
        assert!(uri.contains("def.rs"), "uri should reference def.rs, got: {}", uri);
    }
    hub.shutdown_all();
}

// Test 5: kill mid-flight — request() call itself is non-blocking, Died arrives, respawn works
#[test]
fn test_kill_mid_flight() {
    let root = temp_root();
    let doc_path = root.join("kill.rs");
    std::fs::write(&doc_path, "fn main() {}\n").unwrap();
    let doc_uri = format!("file://{}", doc_path.display());

    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready");

    // The request() call itself must return in <50ms (non-blocking invariant)
    let t0 = Instant::now();
    let _id = hub.request(
        "rust",
        &root,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": {"uri": &doc_uri},
            "position": {"line": 0, "character": 5}
        }),
    );
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(50),
        "request() must return in <50ms, took {:?}",
        elapsed
    );

    // Kill the server
    hub.kill("rust");

    // Died event must arrive
    let died = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Died { .. }),
        Duration::from_millis(500),
    );
    assert!(died.is_some(), "expected Died event after kill");

    // Hub still works: re-ensure + fresh didOpen → fresh Diagnostics
    let ok = hub.ensure("rust", &root, &mock_cmd());
    assert!(ok, "re-ensure after kill should succeed");
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready after respawn");

    hub.notify(
        "rust",
        &root,
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": &doc_uri, "languageId": "rust", "version": 1,
                "text": "fn main() {}\n"
            }
        }),
    );
    let diags = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Diagnostics { .. }),
        Duration::from_millis(500),
    );
    assert!(diags.is_some(), "expected Diagnostics after respawn+didOpen");

    hub.shutdown_all();
}

// Test 6: shutdown_all then request → None, no panic
#[test]
fn test_shutdown_then_request_none() {
    let root = temp_root();
    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready");

    hub.shutdown_all();

    // After shutdown, server is Dead — request returns None
    let result = hub.request(
        "rust",
        &root,
        "textDocument/completion",
        serde_json::json!({}),
    );
    assert!(result.is_none(), "request after shutdown must return None");
    // notify is a no-op (no panic)
    hub.notify("rust", &root, "textDocument/didChange", serde_json::json!({}));
}

// ── Task 4 integration tests ──────────────────────────────────────────────────

use plat::app::App;
use plat::config::{Config, LspServerCfg};

fn mock_config() -> Config {
    let mut cfg = Config::defaults();
    cfg.lsp.insert("rust".to_string(), LspServerCfg {
        command: Some(mock_cmd()),
    });
    cfg
}

fn wait_app_event<F: Fn(&LspEvent) -> bool>(
    app: &mut App,
    pred: F,
    timeout: Duration,
) -> Option<LspEvent> {
    let deadline = Instant::now() + timeout;
    loop {
        app.lsp_pump();
        // drain buffered events
        let mut i = 0;
        while i < app.lsp_events.len() {
            if pred(&app.lsp_events[i]) {
                return Some(app.lsp_events.remove(i));
            }
            i += 1;
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Test 7: App attach → didOpen → 2 Diagnostics
#[test]
fn test_app_attach_did_open_diagnostics() {
    let root = temp_root();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());

    let ev = wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { .. }),
        Duration::from_millis(2000),
    );
    assert!(ev.is_some(), "expected Diagnostics after attach+didOpen");
    if let Some(LspEvent::Diagnostics { diags, .. }) = ev {
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(msgs.contains(&"mock error"), "got: {:?}", msgs);
        assert!(msgs.contains(&"mock warning"), "got: {:?}", msgs);
    }
    app.lsp.shutdown_all();
}

// Test 8: debounced didChange — no sync before 250ms, sync after
#[test]
fn test_debounced_did_change() {
    let root = temp_root();
    let file = root.join("debounce.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());

    // Wait for Ready + initial didOpen Diagnostics
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Edit the doc
    app.docs[0].edit(0, "// edit\n", 0);

    // Drain events that may have arrived during the setup
    app.lsp_pump();
    let _ = app.lsp.take_events();

    // Immediately call lsp_sync_pass — should NOT send (< 250ms)
    app.lsp_sync_pass(std::time::Instant::now());
    std::thread::sleep(Duration::from_millis(20));
    app.lsp_pump();
    let early_events = app.lsp.take_events();
    let has_versioned_early = early_events.iter().any(|e| {
        matches!(e, LspEvent::Diagnostics { diags, .. } if
            diags.iter().any(|d| d.message.contains(" v")))
    });
    assert!(!has_versioned_early, "must NOT sync before debounce: {:?}", early_events);

    // Wait 300ms then call lsp_sync_pass — should send didChange
    std::thread::sleep(Duration::from_millis(300));
    app.lsp_sync_pass(std::time::Instant::now());

    let ev = wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { diags, .. } if
            diags.iter().any(|d| d.message.contains(" v"))),
        Duration::from_millis(1000),
    );
    assert!(ev.is_some(), "expected versioned Diagnostics after debounce");
    app.lsp.shutdown_all();
}

// Test 8b: trailing-edge debounce — ongoing edits reset the 250ms clock
// (review I-1 of Task 4: leading-edge fired mid-typing). Synthetic clock via
// the now: Instant parameter — no extra sleep beyond the negative-check window.
#[test]
fn test_trailing_edge_debounce() {
    let root = temp_root();
    let file = root.join("trailing.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { .. }),
        Duration::from_millis(2000),
    )
    .expect("initial Diagnostics");
    app.lsp_pump();
    let _ = app.lsp.take_events();

    let t0 = Instant::now();
    let at = |ms: u64| t0.checked_add(Duration::from_millis(ms)).unwrap();

    app.docs[0].edit(0, "a", 0);
    app.lsp_sync_pass(at(0)); // first sighting — clock starts
    app.docs[0].edit(1, "b", 0);
    app.lsp_sync_pass(at(150)); // newer gen — clock RESTARTS
    app.lsp_sync_pass(at(300)); // 150ms since last edit: must NOT fire (leading-edge would)

    std::thread::sleep(Duration::from_millis(20));
    app.lsp_pump();
    let early = app.lsp.take_events();
    assert!(
        !early.iter().any(|e| matches!(e, LspEvent::Diagnostics { diags, .. } if
            diags.iter().any(|d| d.message.contains(" v")))),
        "trailing-edge: must not sync while edits continue: {:?}",
        early
    );

    app.lsp_sync_pass(at(400)); // 250ms after the LAST observed edit -> fire
    let ev = wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { diags, .. } if
            diags.iter().any(|d| d.message.contains(" v"))),
        Duration::from_millis(1000),
    );
    assert!(ev.is_some(), "expected versioned Diagnostics 250ms after last edit");
    app.lsp.shutdown_all();
}

// Test 9: lsp_sync_now bypasses debounce
#[test]
fn test_lsp_sync_now_no_debounce() {
    let root = temp_root();
    let file = root.join("syncnow.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());

    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Edit then immediately force sync (no sleep)
    app.docs[0].edit(0, "// sync now\n", 0);
    app.lsp_sync_now(0);

    let ev = wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { diags, .. } if
            diags.iter().any(|d| d.message.contains(" v"))),
        Duration::from_millis(1000),
    );
    assert!(ev.is_some(), "lsp_sync_now must send immediately without debounce");
    app.lsp.shutdown_all();
}

// Test 10: no LSP server for unknown language → zero spawn, zero events
#[test]
fn test_no_server_unknown_lang() {
    let root = temp_root();
    let file = root.join("script.xyz");
    std::fs::write(&file, "content").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());

    std::thread::sleep(Duration::from_millis(100));
    app.lsp_pump();
    let events = app.lsp.take_events();
    assert!(events.is_empty(), "unknown lang must produce zero events: {:?}", events);
    assert!(app.lsp_open.is_empty(), "unknown lang must not be opened");
}

// Test 11: path with space — %20 encoding works end-to-end
#[test]
fn test_path_with_space_encoding() {
    let root = temp_root();
    let spaced = root.join("my file.rs");
    std::fs::write(&spaced, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![spaced.clone()], mock_config());

    // Should still get Diagnostics (mock decodes %20 in classify_message)
    let ev = wait_app_event(
        &mut app,
        |e| matches!(e, LspEvent::Diagnostics { .. }),
        Duration::from_millis(2000),
    );
    assert!(ev.is_some(), "space in path must not prevent Diagnostics");
    app.lsp.shutdown_all();
}

// Test 12: end-to-end diagnostics UX (Task 5):
//   - pump stores doc.lsp_diags with char coords
//   - status bar renders error+warning counts
//   - cursor on diag line shows message in status
//   - ] navigation lands on the right diag lines
#[test]
fn test_diag_ux_end_to_end() {
    use plat::render::draw;
    use ratatui::backend::TestBackend;

    let root = temp_root();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {}\nlet x = 1;\nlet y = 2;\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());

    // Wait until diags are stored in the doc
    let deadline = std::time::Instant::now() + Duration::from_millis(2000);
    loop {
        app.lsp_pump();
        if !app.docs[0].lsp_diags.is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for lsp_diags to be populated");
        std::thread::sleep(Duration::from_millis(10));
    }

    let diags = &app.docs[0].lsp_diags;
    assert_eq!(diags.len(), 2, "two diags stored: {:?}", diags);
    // mock: error@line0, warning@line1 — 0-based
    assert!(diags.iter().any(|d| d.line == 0 && d.severity == 1), "error on line 0: {:?}", diags);
    assert!(diags.iter().any(|d| d.line == 1 && d.severity == 2), "warning on line 1: {:?}", diags);

    // Status bar shows counts
    let buf = {
        let mut term = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        term.backend().buffer().clone()
    };
    let status: String = (0..80u16).map(|x| buf[(x, 22)].symbol().chars().next().unwrap_or(' ')).collect();
    assert!(status.contains("1E"), "status must show 1E: {status:?}");
    assert!(status.contains("1W"), "status must show 1W: {status:?}");

    // Cursor on line 0 (error line) shows message on the bottom echo line
    let echo: String = (0..80u16).map(|x| buf[(x, 23)].symbol().chars().next().unwrap_or(' ')).collect();
    assert!(echo.contains("mock error"), "cursor on diag line shows message on echo line: {echo:?}");

    // ] navigation: from line 0, ] → line 1 (warning)
    app.handle_command(plat::keymap::Command::NextDiagnostic);
    let cur_line = app.docs[0].rope.char_to_line(app.docs[0].cursor);
    assert_eq!(cur_line, 1, "] from line 0 should jump to line 1 (next diag)");

    // ] again wraps to line 0
    app.handle_command(plat::keymap::Command::NextDiagnostic);
    let cur_line = app.docs[0].rope.char_to_line(app.docs[0].cursor);
    assert_eq!(cur_line, 0, "] from last diag wraps to line 0");

    app.lsp.shutdown_all();
}


// ── Task 6: Completion flow tests ─────────────────────────────────────────────


use plat::keymap::Command;
use crossterm::event::{KeyCode, KeyModifiers, KeyEvent, KeyEventKind, KeyEventState};

fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

fn wait_completion(app: &mut App, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        app.lsp_pump();
        if app.completion.is_some() { return true; }
        if Instant::now() >= deadline { return false; }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Test: Ctrl-Space triggers completion, Tab moves sel, Enter inserts alpha_full
#[test]
fn test_completion_ctrl_space_tab_enter() {
    let root = temp_root();
    let file = root.join("comp6.rs");
    std::fs::write(&file, "alp\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Position cursor at end of "alp"
    app.docs[0].cursor = 3;
    app.handle_command(Command::Completion);

    let opened = wait_completion(&mut app, Duration::from_millis(2000));
    assert!(opened, "completion menu should open after Ctrl-Space");

    let menu = app.completion.as_ref().unwrap();
    assert!(!menu.items.is_empty(), "menu must have items");
    // First item should be alpha_full (kind: ?)
    assert!(menu.items.iter().any(|i| i.label == "alpha_full"),
        "alpha_full missing from items: {:?}", menu.items.iter().map(|i| &i.label).collect::<Vec<_>>());

    // Tab -> sel 1
    app.handle_completion_key(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.completion.as_ref().unwrap().sel, 1);

    // Go back to sel 0 for alpha_full
    app.handle_completion_key(&press(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.completion.as_ref().unwrap().sel, 0);

    // Enter -> accept alpha_full
    app.handle_completion_key(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.completion.is_none(), "menu must close after Enter");

    let content = app.docs[0].rope.to_string();
    assert!(content.contains("alpha_full"),
        "alpha_full should be in buffer: {content:?}");

    app.lsp.shutdown_all();
}

// Test: stale request (doc edited after request) -> menu never opens
#[test]
fn test_stale_completion_response_dropped() {
    let root = temp_root();
    let file = root.join("stale6.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Trigger completion
    app.docs[0].cursor = 3;
    app.handle_command(Command::Completion);
    assert!(app.pending_completion.is_some(), "pending should be set");

    // Edit doc before response arrives (bump gen)
    app.docs[0].edit(0, "// stale\n", 0);

    // Now pump until response arrives or timeout
    let deadline = Instant::now() + Duration::from_millis(2000);
    loop {
        app.lsp_pump();
        if app.completion.is_some() { break; }
        if Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(app.completion.is_none(), "stale completion must not open menu");
    app.lsp.shutdown_all();
}

// Test: Tab on identifier char triggers completion when LSP configured
#[test]
fn test_smart_tab_on_identifier_triggers_completion() {
    let root = temp_root();
    let file = root.join("smart6.rs");
    std::fs::write(&file, "alp\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Cursor after identifier char
    app.docs[0].cursor = 3; // after "alp"
    app.handle_command(Command::Indent); // smart-tab triggers completion

    // pending_completion set (server configured) — menu opens after pump
    let opened = wait_completion(&mut app, Duration::from_millis(2000));
    assert!(opened, "smart-tab on identifier should trigger completion");

    app.lsp.shutdown_all();
}

// Test: Tab on blank line still indents (no LSP for that file's lang)
#[test]
fn test_tab_on_blank_line_indents_no_lsp() {
    let root = temp_root();
    let file = root.join("blank.xyz"); // unknown lang -> no LSP
    std::fs::write(&file, "\n").unwrap();

    // Use default config (no LSP for .xyz)
    let mut app = App::new(vec![file.clone()]);
    app.docs[0].cursor = 0;
    let before_gen = app.docs[0].edit_gen;
    app.handle_command(Command::Indent);

    // Should indent (not trigger completion)
    assert!(app.completion.is_none(), "no completion for unknown lang");
    // Doc should have been modified (indent inserted)
    assert_ne!(app.docs[0].edit_gen, before_gen, "indent should have bumped gen");
    let content = app.docs[0].rope.to_string();
    assert!(content.starts_with("  ") || content.starts_with('\t'),
        "should have indent: {content:?}");
}

// ── Task 7: goto-definition + jump-back flow tests ────────────────────────────

fn wait_goto(app: &mut App, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    // Wait until pending_goto is cleared (response consumed)
    loop {
        app.lsp_pump();
        if app.pending_goto.is_none() { return true; }
        if Instant::now() >= deadline { return false; }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn task7_root(suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lsp_t7_{}_{}", std::process::id(), suffix));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_goto_def_ctrl_bracket_lands_line_0() {
    let root = task7_root("goto");
    let file = root.join("goto7.rs");
    // Put cursor on line 2
    std::fs::write(&file, "line0\nline1\nline2\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Move cursor to line 2
    app.docs[0].cursor = app.docs[0].rope.line_to_char(2);
    let orig_cursor = app.docs[0].cursor;
    assert_eq!(app.docs[0].rope.char_to_line(orig_cursor), 2);

    // Trigger GotoDef (mock returns Location at line 0, col 0 of same file)
    app.handle_command(Command::GotoDef);
    assert!(app.pending_goto.is_some(), "pending_goto set");

    let consumed = wait_goto(&mut app, Duration::from_millis(2000));
    assert!(consumed, "goto response consumed");

    // cursor should be on line 0 (mock location 0,0)
    let cur_line = app.docs[0].rope.char_to_line(app.docs[0].cursor);
    assert_eq!(cur_line, 0, "cursor landed on line 0");

    // Jump stack has one entry
    assert_eq!(app.jump_stack.len(), 1);

    // g o (JumpBack) returns to original cursor
    app.handle_command(Command::JumpBack);
    assert_eq!(app.docs[0].cursor, orig_cursor, "jumped back to original cursor");
    assert_eq!(app.jump_stack.len(), 0, "stack empty after jump back");

    app.lsp.shutdown_all();
}

#[test]
fn test_goto_def_with_completion_menu_open() {
    // Ctrl-] while completion menu open: menu closes AND goto fires
    let root = task7_root("menu");
    let file = root.join("goto7menu.rs");
    std::fs::write(&file, "alp\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    // Open completion menu
    app.docs[0].cursor = 3;
    app.handle_command(Command::Completion);
    let opened = wait_completion(&mut app, Duration::from_millis(2000));
    assert!(opened, "completion menu open");

    // Send Ctrl-] through handle_completion_key (other→close+route)
    let ctrl_bracket = press(KeyCode::Char(']'), KeyModifiers::CONTROL);
    app.handle_completion_key(&ctrl_bracket);

    // Menu must be closed
    assert!(app.completion.is_none(), "menu closed by Ctrl-]");
    // pending_goto must be set (goto fired)
    assert!(app.pending_goto.is_some(), "pending_goto set after Ctrl-]");

    // Consume the goto response
    wait_goto(&mut app, Duration::from_millis(2000));
    let cur_line = app.docs[0].rope.char_to_line(app.docs[0].cursor);
    assert_eq!(cur_line, 0, "goto applied: cursor on line 0");

    app.lsp.shutdown_all();
}

// Test: textDocument/formatting → TextEdit[] from mock
#[test]
fn test_formatting_request() {
    let root = temp_root();
    let doc_path = root.join("fmt.rs");
    std::fs::write(&doc_path, "fn main() {}\n").unwrap();
    let doc_uri = format!("file://{}", doc_path.display());

    let mut hub = LspHub::default();
    hub.ensure("rust", &root, &mock_cmd());
    wait_event(&mut hub, |e| matches!(e, LspEvent::Ready { .. }), Duration::from_millis(500))
        .expect("Ready");

    hub.notify(
        "rust",
        &root,
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": &doc_uri, "languageId": "rust", "version": 1,
                "text": "fn main() {}\n"
            }
        }),
    );

    let id = hub
        .request(
            "rust",
            &root,
            "textDocument/formatting",
            serde_json::json!({
                "textDocument": {"uri": &doc_uri},
                "options": {"tabSize": 2, "insertSpaces": true}
            }),
        )
        .expect("request returns Some(id)");

    let ev = wait_event(
        &mut hub,
        |e| matches!(e, LspEvent::Response { id: eid, .. } if *eid == id),
        Duration::from_millis(2000),
    )
    .expect("formatting Response");

    if let LspEvent::Response { result, .. } = ev {
        let arr = result.as_array().expect("result is a TextEdit array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["newText"], "// fmt\n");
    }
    hub.shutdown_all();
}

// --- #9 code actions ---------------------------------------------------------

fn wait_code_actions(app: &mut App, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        app.lsp_pump();
        if app.code_actions.is_some() { return true; }
        if Instant::now() >= deadline { return false; }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Test: CodeAction opens the mock menu; the title proves context.diagnostics
// round-tripped; Enter applies the edit as ONE undo batch.
#[test]
fn test_code_action_menu_apply_undo() {
    let root = temp_root();
    let file = root.join("ca9.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    app.handle_command(Command::CodeAction);
    assert!(wait_code_actions(&mut app, Duration::from_millis(2000)), "action menu should open");

    let menu = app.code_actions.as_ref().unwrap();
    assert!(
        menu.items.iter().any(|i| i.title == "mock: fix 2 diagnostics"),
        "diagnostics context must round-trip, got {:?}",
        menu.items.iter().map(|i| &i.title).collect::<Vec<_>>()
    );

    // Enter applies the mock's quickfix (line 0, cols 0..1 → "X").
    app.handle_code_action_key(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.docs[0].rope.to_string(), "Xn main() {}\n");
    assert!(app.docs[0].dirty, "manual action leaves the buffer dirty on purpose");
    assert!(app.code_actions.is_none(), "menu closes after accepting");

    // One undo restores everything (single batch).
    app.handle_command(Command::Undo);
    assert_eq!(app.docs[0].rope.to_string(), "fn main() {}\n", "one undo restores the whole action");
}

// Test: the second mock action has no edit (command-only) → rejected with a
// status note, text untouched.
#[test]
fn test_code_action_command_only_action_rejected() {
    let root = temp_root();
    let file = root.join("ca9cmd.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    app.handle_command(Command::CodeAction);
    assert!(wait_code_actions(&mut app, Duration::from_millis(2000)), "action menu should open");

    app.handle_code_action_key(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.code_actions.as_ref().unwrap().sel, 1, "Tab moves selection");
    app.handle_code_action_key(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.status.as_ref().map(|s| s.0.contains("command")) == Some(true),
        "command-only action must report unsupported, got {:?}", app.status
    );
    assert_eq!(app.docs[0].rope.to_string(), "fn main() {}\n", "text untouched");
}

// Test: user edits before the response lands → stale response dropped, no menu.
#[test]
fn test_code_action_stale_response_dropped() {
    let root = temp_root();
    let file = root.join("ca9stale.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = App::new_with_config(vec![file.clone()], mock_config());
    wait_app_event(&mut app, |e| matches!(e, LspEvent::Diagnostics { .. }), Duration::from_millis(2000))
        .expect("initial Diagnostics");

    app.handle_command(Command::CodeAction);
    app.docs[0].edit(1, "Z", 0); // user types immediately

    assert!(
        !wait_code_actions(&mut app, Duration::from_millis(500)),
        "stale response must not open the menu"
    );
    assert!(app.pending_code_action.is_none(), "pending must be consumed");
}
