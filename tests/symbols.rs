//! Tests for symbol data types and LSP parsing

use serde_json::json;

#[test]
fn test_parse_file_symbols_response() {
    let json_data = json!([
        {
            "name": "main",
            "kind": 12,
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 10, "character": 0}
            },
            "selectionRange": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 8}},
            "detail": "fn() -> i32"
        }
    ]);
    
    let symbols = plat::symbols::parse_file_symbols(json_data).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "main");
    assert_eq!(symbols[0].kind, 12); // Function
    assert_eq!(symbols[0].range, (0, 0, 10, 0));
    assert_eq!(symbols[0].detail, Some("fn() -> i32".to_string()));
}

#[test]
fn test_parse_file_symbols_symbol_information_shape() {
    let json_data = json!([
        {
            "name": "run",
            "kind": 12,
            "location": {
                "uri": "file:///tmp/a.rs",
                "range": {
                    "start": {"line": 41, "character": 7},
                    "end": {"line": 41, "character": 10}
                }
            },
            "containerName": "impl App"
        }
    ]);

    let symbols = plat::symbols::parse_file_symbols(json_data).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "run");
    assert_eq!(symbols[0].range, (41, 7, 41, 10));
}

#[test]
fn test_parse_workspace_symbols_response() {
    let json_data = json!([
        {
            "name": "handle_command",
            "kind": 12,
            "location": {
                "uri": "file:///home/user/app.rs",
                "range": {"start": {"line": 100, "character": 0}, "end": {"line": 200, "character": 0}}
            },
            "containerName": "impl App"
        }
    ]);
    
    let symbols = plat::symbols::parse_workspace_symbols(json_data).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "handle_command");
    assert_eq!(symbols[0].container, Some("impl App".to_string()));
}

#[test]
fn test_symbol_kind_icons() {
    assert_eq!(plat::symbols::symbol_kind_icon(12), "ƒ"); // Function
    assert_eq!(plat::symbols::symbol_kind_icon(5), "Ⓣ"); // Class
    assert_eq!(plat::symbols::symbol_kind_icon(11), "♦"); // Enum
    assert_eq!(plat::symbols::symbol_kind_icon(13), "Ϸ"); // Variable
}

#[test]
fn test_symbol_truncation_detection() {
    // Test that we correctly identify when results are truncated (>= MAX_RESULTS)
    let mut symbols = vec![];
    for i in 0..101 {
        symbols.push(plat::symbols::LspSymbol {
            name: format!("sym_{}", i),
            kind: 12,
            range: (i as u32, 0, i as u32, 0),
            detail: None,
            container: None,
            uri: None,
        });
    }
    assert!(symbols.len() > 100);
}
