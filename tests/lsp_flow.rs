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
    let dir = std::env::temp_dir().join(format!("lsp_test_{}", std::process::id()));
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

