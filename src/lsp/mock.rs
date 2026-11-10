use std::io::{BufReader, stdin, stdout};
use serde_json::{json, Value};
use crate::lsp::transport::{write_message, Frmr};

/// Deterministic mock LSP server. Run via `PLAT_LSP_MOCK=1 plat`.
/// No threads, no sleeps. Serves requests synchronously in a loop.
pub fn serve() {
    let stdin = stdin();
    let stdout = stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut framer = Frmr::new();
    let mut _last_opened_uri: Option<String> = None;

    loop {
        let msg = match framer.read_message(&mut reader) {
            Ok(Some(m)) => m,
            _ => break, // EOF or poison → exit
        };

        let method = msg.get("method").and_then(|v| v.as_str());
        let id = msg.get("id").cloned();

        match method {
            Some("initialize") => {
                let result = json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "completionProvider": {},
                        "definitionProvider": true,
                        "documentFormattingProvider": true,
                        "codeActionProvider": true
                    },
                    "serverInfo": { "name": "plat-mock", "version": "0.1" }
                });
                send_response(&mut writer, &id.unwrap(), result);
            }
            Some("initialized") => {
                // no-op notification
            }
            Some("textDocument/didOpen") => {
                if let Some(uri) = msg["params"]["textDocument"]["uri"].as_str() {
                    _last_opened_uri = Some(uri.to_string());
                    // Push diagnostics for opened doc
                    let diags = json!([
                        {
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            },
                            "severity": 1,
                            "message": "mock error"
                        },
                        {
                            "range": {
                                "start": {"line": 1, "character": 0},
                                "end": {"line": 1, "character": 1}
                            },
                            "severity": 2,
                            "message": "mock warning"
                        }
                    ]);
                    send_notification(
                        &mut writer,
                        "textDocument/publishDiagnostics",
                        json!({ "uri": uri, "diagnostics": diags }),
                    );
                }
            }
            Some("textDocument/didChange") => {
                // Push diagnostics with version suffix so tests can observe sync
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                let version = msg["params"]["textDocument"]["version"].as_i64().unwrap_or(0);
                if !uri.is_empty() {
                    let diags = json!([
                        {
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 1}
                            },
                            "severity": 1,
                            "message": format!("mock error v{}", version)
                        },
                        {
                            "range": {
                                "start": {"line": 1, "character": 0},
                                "end": {"line": 1, "character": 1}
                            },
                            "severity": 2,
                            "message": format!("mock warning v{}", version)
                        }
                    ]);
                    send_notification(
                        &mut writer,
                        "textDocument/publishDiagnostics",
                        json!({ "uri": uri, "diagnostics": diags }),
                    );
                }
            }
            Some("textDocument/completion") => {
                let pos = &msg["params"]["position"];
                let line = pos["line"].as_u64().unwrap_or(0);
                let character = pos["character"].as_u64().unwrap_or(0);
                let start_char = character.saturating_sub(3);
                let result = json!([
                    {
                        "label": "alpha_full"
                    },
                    {
                        "label": "beta_x(${1:arg})",
                        "insertTextFormat": 2,
                        "insertText": "beta_x(${1:arg})"
                    },
                    {
                        "label": "gamma",
                        "textEdit": {
                            "range": {
                                "start": {"line": line, "character": start_char},
                                "end": {"line": line, "character": character}
                            },
                            "newText": "gamma_edit"
                        }
                    }
                ]);
                send_response(&mut writer, &id.unwrap(), result);
            }
            Some("textDocument/definition") => {
                let uri = msg["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let result = json!({
                    "uri": uri,
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 0}
                    }
                });
                send_response(&mut writer, &id.unwrap(), result);
            }
            Some("textDocument/formatting") => {
                let result = json!([
                    {
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0}
                        },
                        "newText": "// fmt\n"
                    }
                ]);
                send_response(&mut writer, &id.unwrap(), result);
            }
            Some("textDocument/codeAction") => {
                let uri = msg["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                // Echo the diagnostics context count into the title: tests assert
                // the count matches the diagnostics plat pushed, proving the
                // context round-trips. No context diagnostics → no actions.
                let n = msg["params"]["context"]["diagnostics"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                if n == 0 {
                    send_response(&mut writer, &id.unwrap(), json!([]));
                } else {
                    let result = json!([
                        {
                            "title": format!("mock: fix {n} diagnostics"),
                            "kind": "quickfix",
                            "edit": {
                                "changes": {
                                    &uri: [{
                                        "range": {
                                            "start": {"line": 0, "character": 0},
                                            "end": {"line": 0, "character": 1}
                                        },
                                        "newText": "X"
                                    }]
                                }
                            }
                        },
                        {
                            // command-only action: no edit → plat must report
                            // it as unsupported rather than silently doing nothing
                            "title": "mock: run a command",
                            "kind": "source",
                            "command": { "title": "mock", "command": "mock.noop" }
                        }
                    ]);
                    send_response(&mut writer, &id.unwrap(), result);
                }
            }
            Some("shutdown") => {
                send_response(&mut writer, &id.unwrap(), Value::Null);
            }
            Some("exit") => {
                std::process::exit(0);
            }
            _ => {
                // Unknown method with id → empty result
                if let Some(req_id) = id {
                    send_response(&mut writer, &req_id, json!({}));
                }
            }
        }
    }
}

fn send_response(w: &mut impl std::io::Write, id: &Value, result: Value) {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    });
    let _ = write_message(w, &msg);
}

fn send_notification(w: &mut impl std::io::Write, method: &str, params: Value) {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    let _ = write_message(w, &msg);
}
